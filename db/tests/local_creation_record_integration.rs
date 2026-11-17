//! Local creation records are immutable groundwork, not an authoritative sync frontier.
mod common;
use db::{
    blob_db,
    causal_envelope::{self, Action, CreationCompleteness, EntityKind},
    db_context::DbContext,
    group_db,
    model::user::User,
    model_db, random_hex_32, user_db,
};

async fn record(
    db: &DbContext,
    user: &User,
    kind: EntityKind,
    birth: &str,
) -> causal_envelope::LocalCreationRecord {
    let value = causal_envelope::get_local_creation_record(db, user, kind, birth)
        .await
        .unwrap();
    assert!(
        value.is_some(),
        "new local entity must have an isolated creation record"
    );
    value.unwrap()
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn new_local_births_have_stable_owner_scoped_incomplete_creation_records() {
    let (dir, db) = common::test_db().await;
    let user = User::default();
    let foreign_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let foreign = user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, "created-model", "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "part", blob, None, None)
        .await
        .unwrap();
    let group_birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let (model_global, model_birth): (String, String) = sqlx::query_as(
        "SELECT model_unique_global_id, model_birth_id FROM models WHERE model_id = ?",
    )
    .bind(model)
    .fetch_one(&db)
    .await
    .unwrap();
    let group_record = record(&db, &user, EntityKind::Group, &group_birth).await;
    let model_record = record(&db, &user, EntityKind::Model, &model_birth).await;
    assert_eq!(group_record.completeness, CreationCompleteness::Incomplete);
    assert_eq!(model_record.completeness, CreationCompleteness::Incomplete);
    assert_eq!(group_record.envelope.entity.kind, EntityKind::Group);
    assert_eq!(
        group_record.envelope.entity.global_id,
        group.unique_global_id
    );
    assert_eq!(
        group_record.envelope.entity.birth_id.as_deref(),
        Some(group_birth.as_str())
    );
    assert_eq!(
        group_record.envelope.action,
        Action::GroupCreated {
            name: "folder".into()
        }
    );
    assert_eq!(model_record.envelope.entity.kind, EntityKind::Model);
    assert_eq!(model_record.envelope.entity.global_id, model_global);
    assert_eq!(
        model_record.envelope.entity.birth_id.as_deref(),
        Some(model_birth.as_str())
    );
    assert_eq!(model_record.envelope.action, Action::ModelCreated);
    for event in [&group_record.envelope, &model_record.envelope] {
        assert_eq!(event.version, 2);
        assert_eq!(event.parents, []);
        assert_eq!(event.dependencies, []);
        let wire = causal_envelope::canonical_causal_envelope(event).unwrap();
        assert_eq!(
            causal_envelope::parse_causal_envelope(&wire).unwrap(),
            *event
        );
    }
    assert_ne!(group_record.envelope.event, model_record.envelope.event);
    assert!(
        causal_envelope::get_local_creation_record(&db, &foreign, EntityKind::Group, &group_birth)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        causal_envelope::get_local_creation_record(&db, &user, EntityKind::Model, &group_birth)
            .await
            .unwrap()
            .is_none()
    );
    group_db::edit_group(&db, &user, group.id, "renamed", None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    let after_group = record(&db, &user, EntityKind::Group, &group_birth).await;
    let after_model = record(&db, &user, EntityKind::Model, &model_birth).await;
    assert_eq!(after_group.completeness, CreationCompleteness::Incomplete);
    assert_eq!(after_model.completeness, CreationCompleteness::Incomplete);
    assert_eq!(after_group.envelope, group_record.envelope);
    assert_eq!(after_model.envelope, model_record.envelope);
    db.close().await;
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    assert_eq!(
        record(&db, &user, EntityKind::Group, &group_birth)
            .await
            .envelope,
        group_record.envelope
    );
    assert_eq!(
        record(&db, &user, EntityKind::Model, &model_birth)
            .await
            .envelope,
        model_record.envelope
    );
}

#[tokio::test]
async fn record_insert_failure_rolls_back_entity_birth_and_dot_allocation() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let blob = blob_db::add_blob(&db, "creation-fail", "stl", 1, None)
        .await
        .unwrap();
    let counter: i64 =
        sqlx::query_scalar("SELECT counter FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    let highwater: Vec<(String, i64)> = sqlx::query_as(
        "SELECT object_kind, highwater FROM object_id_highwater ORDER BY object_kind",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER reject_local_creation BEFORE INSERT ON local_creation_records BEGIN SELECT RAISE(ABORT, 'injected creation record failure'); END").execute(&db).await.unwrap();
    assert!(
        group_db::add_empty_group(&db, &user, "failure", None)
            .await
            .is_err()
    );
    assert!(
        model_db::add_model(&db, &user, "failure", blob, None, None)
            .await
            .is_err()
    );
    let counts: (i64, i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group), (SELECT COUNT(*) FROM models), (SELECT COUNT(*) FROM entity_births), (SELECT COUNT(*) FROM local_creation_records)").fetch_one(&db).await.unwrap();
    assert_eq!(counts, (0, 0, 0, 0));
    let after: i64 =
        sqlx::query_scalar("SELECT counter FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(after, counter);
    let after_highwater: Vec<(String, i64)> = sqlx::query_as(
        "SELECT object_kind, highwater FROM object_id_highwater ORDER BY object_kind",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(after_highwater, highwater);
    sqlx::query("DROP TRIGGER reject_local_creation")
        .execute(&db)
        .await
        .unwrap();
    assert!(
        group_db::add_empty_group(&db, &user, "success", None)
            .await
            .is_ok()
    );
    assert!(
        model_db::add_model(&db, &user, "success", blob, None, None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn migration_does_not_fabricate_genesis_for_existing_known_or_unknown_births() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version < 20_261_113_020_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let group_birth = random_hex_32();
    let model_birth = random_hex_32();
    let group = sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id, group_birth_id) VALUES ('existing', '2020-01-01T00:00:00Z', ?, '2020-01-01T00:00:00Z', ?, ?)").bind(user.id).bind(random_hex_32()).bind(&group_birth).execute(&db).await.unwrap().last_insert_rowid();
    let unknown = common::legacy_group(&db, &user, "unknown").await;
    let blob = blob_db::add_blob(&db, "old-record", "stl", 1, None)
        .await
        .unwrap();
    sqlx::query("INSERT INTO models (model_id, model_name, model_blob_id, model_added, model_user_id, model_unique_global_id, model_last_modified, model_birth_id) VALUES (10, 'existing', ?, '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z', ?)").bind(blob).bind(user.id).bind(random_hex_32()).bind(&model_birth).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO models (model_id, model_name, model_blob_id, model_added, model_user_id, model_unique_global_id, model_last_modified) VALUES (11, 'unknown', ?, '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')").bind(blob).bind(user.id).bind(random_hex_32()).execute(&db).await.unwrap();
    let global: String =
        sqlx::query_scalar("SELECT group_unique_global_id FROM models_group WHERE group_id = ?")
            .bind(group)
            .fetch_one(&db)
            .await
            .unwrap();
    let writer = random_hex_32();
    sqlx::query("UPDATE models_group SET group_name = 'already renamed' WHERE group_id = ?")
        .bind(group)
        .execute(&db)
        .await
        .unwrap();
    let event = sqlx::query("INSERT INTO group_causal_events (user_id,group_global_id,replica_id,counter,name) VALUES (?,?,?,1,'already renamed')")
        .bind(user.id).bind(&global).bind(&writer).execute(&db).await.unwrap().last_insert_rowid();
    sqlx::query(
        "INSERT INTO group_causal_frontier (user_id,group_global_id,event_id) VALUES (?,?,?)",
    )
    .bind(user.id)
    .bind(&global)
    .bind(event)
    .execute(&db)
    .await
    .unwrap();
    db.close().await;
    let db = db::db_context::setup_db(&path, &dir.path().join("backups")).await;
    assert!(
        causal_envelope::get_local_creation_record(&db, &user, EntityKind::Group, &group_birth)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        causal_envelope::get_local_creation_record(&db, &user, EntityKind::Model, &model_birth)
            .await
            .unwrap()
            .is_none()
    );
    let old_group_birth: Option<String> =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(unknown.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let old_model_birth: Option<String> =
        sqlx::query_scalar("SELECT model_birth_id FROM models WHERE model_id = 11")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!((old_group_birth, old_model_birth), (None, None));
    let records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_creation_records")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(records, 0);
}
