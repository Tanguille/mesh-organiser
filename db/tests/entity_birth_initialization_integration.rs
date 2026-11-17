//! A known portable birth does not imply a known initialization event.
mod common;
use db::{
    blob_db,
    causal_envelope::{self, Action, CausalEnvelope, Dot, Entity, EntityKind},
    db_context::DbContext,
    group_db,
    model::{ModelFlags, user::User},
    model_db, random_hex_32, user_db,
};

async fn assert_schema(db: &DbContext) {
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('entity_births')")
            .fetch_all(db)
            .await
            .unwrap();
    for name in [
        "initialization_status",
        "initialization_writer_id",
        "initialization_counter",
    ] {
        assert!(
            columns.iter().any(|column| column == name),
            "missing entity_births.{name}"
        );
    }
}
async fn initialization(
    db: &DbContext,
    user_id: i64,
    kind: &str,
    birth: &str,
) -> Option<(String, Option<String>, Option<i64>)> {
    sqlx::query_as("SELECT initialization_status, initialization_writer_id, initialization_counter FROM entity_births WHERE user_id = ? AND entity_kind = ? AND birth_id = ?")
        .bind(user_id).bind(kind).bind(birth).fetch_optional(db).await.unwrap()
}

async fn rekey_model(db: &DbContext, user: &User, id: i64, global: &str) {
    model_db::edit_model(
        db,
        user,
        id,
        "part",
        None,
        None,
        ModelFlags::empty(),
        None,
        Some(global),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn new_births_reference_exact_creation_events_and_remain_owner_scoped() {
    let (dir, db) = common::test_db().await;
    assert_schema(&db).await;
    let user = User::default();
    let foreign_id = user_db::add_user(&db, "foreign", "foreign@example.test", &random_hex_32())
        .await
        .unwrap();
    let foreign = user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let other = group_db::add_empty_group(&db, &foreign, "foreign", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, "initialization-model", "stl", 1, None)
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
    let other_birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(other.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let model_birth: String =
        sqlx::query_scalar("SELECT model_birth_id FROM models WHERE model_id = ?")
            .bind(model)
            .fetch_one(&db)
            .await
            .unwrap();
    for (kind, label, birth) in [
        (EntityKind::Group, "group", &group_birth),
        (EntityKind::Model, "model", &model_birth),
    ] {
        let record = causal_envelope::get_local_creation_record(&db, &user, kind, birth)
            .await
            .unwrap()
            .unwrap();
        let dot = record.envelope.event;
        assert_eq!(
            initialization(&db, user.id, label, birth).await,
            Some((
                "created".into(),
                Some(dot.writer_id),
                Some(dot.counter.parse().unwrap())
            ))
        );
        assert_eq!(initialization(&db, foreign.id, label, birth).await, None);
    }
    assert_eq!(
        initialization(&db, user.id, "group", &other_birth).await,
        None
    );
    let expected = initialization(&db, user.id, "group", &group_birth).await;
    group_db::edit_group(&db, &user, group.id, "renamed", None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    assert_eq!(
        initialization(&db, user.id, "group", &group_birth).await,
        expected
    );
    db.close().await;
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    assert_eq!(
        initialization(&db, user.id, "group", &group_birth).await,
        expected
    );
}

#[tokio::test]
async fn failed_initialization_update_rolls_back_creation_record_entity_and_counter() {
    let (_dir, db) = common::test_db().await;
    assert_schema(&db).await;
    let user = User::default();
    let blob = blob_db::add_blob(&db, "init-failure", "stl", 1, None)
        .await
        .unwrap();
    let before: (String, i64) =
        sqlx::query_as("SELECT replica_id, counter FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    sqlx::query("CREATE TRIGGER reject_initialization BEFORE UPDATE OF initialization_status ON entity_births WHEN NEW.initialization_status = 'created' BEGIN SELECT RAISE(ABORT, 'injected failure'); END").execute(&db).await.unwrap();
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
    let after: (String, i64) =
        sqlx::query_as("SELECT replica_id, counter FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn initialization_cannot_reference_a_different_entity_or_inconsistent_dot() {
    let (_dir, db) = common::test_db().await;
    assert_schema(&db).await;
    let user = User::default();
    let one = group_db::add_empty_group(&db, &user, "one", None)
        .await
        .unwrap();
    let two = group_db::add_empty_group(&db, &user, "two", None)
        .await
        .unwrap();
    let first_birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(one.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let second_birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(two.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let record =
        causal_envelope::get_local_creation_record(&db, &user, EntityKind::Group, &second_birth)
            .await
            .unwrap()
            .unwrap();
    let dot = record.envelope.event;
    let wrong = sqlx::query("UPDATE entity_births SET initialization_status = 'created', initialization_writer_id = ?, initialization_counter = ? WHERE user_id = ? AND entity_kind = 'group' AND birth_id = ?").bind(dot.writer_id).bind(dot.counter.parse::<i64>().unwrap()).bind(user.id).bind(&first_birth).execute(&db).await;
    assert!(
        wrong.is_err(),
        "a creation event from a different birth must not be accepted"
    );
    let incomplete = sqlx::query("UPDATE entity_births SET initialization_status = 'created', initialization_writer_id = NULL WHERE user_id = ? AND entity_kind = 'group' AND birth_id = ?").bind(user.id).bind(&first_birth).execute(&db).await;
    assert!(incomplete.is_err());
}

#[tokio::test]
async fn migration_keeps_known_births_with_old_creation_records_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version < 20_261_113_060_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let group_global = random_hex_32();
    let group_birth = random_hex_32();
    let model_global = random_hex_32();
    let model_birth = random_hex_32();
    sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id, group_birth_id) VALUES ('existing', '2020-01-01T00:00:00Z', ?, '2020-01-01T00:00:00Z', ?, ?)").bind(user.id).bind(&group_global).bind(&group_birth).execute(&db).await.unwrap();
    let blob = blob_db::add_blob(&db, "old-initialization", "stl", 1, None)
        .await
        .unwrap();
    sqlx::query("INSERT INTO models (model_id, model_name, model_blob_id, model_added, model_user_id, model_unique_global_id, model_last_modified, model_birth_id) VALUES (1, 'existing', ?, '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z', ?)").bind(blob).bind(user.id).bind(&model_global).bind(&model_birth).execute(&db).await.unwrap();
    let writer = random_hex_32();
    for (kind, global, birth, counter, action) in [
        (
            EntityKind::Group,
            &group_global,
            &group_birth,
            1,
            Action::GroupCreated {
                name: "existing".into(),
            },
        ),
        (
            EntityKind::Model,
            &model_global,
            &model_birth,
            2,
            Action::ModelCreated,
        ),
    ] {
        let label = if kind == EntityKind::Group {
            "group"
        } else {
            "model"
        };
        let envelope = CausalEnvelope {
            version: 2,
            entity: Entity {
                kind,
                global_id: global.clone(),
                birth_id: Some(birth.clone()),
            },
            event: Dot {
                writer_id: writer.clone(),
                counter: counter.to_string(),
            },
            parents: vec![],
            dependencies: vec![],
            action,
        };
        let canonical = causal_envelope::canonical_causal_envelope(&envelope).unwrap();
        sqlx::query("INSERT INTO local_creation_records (user_id, entity_kind, birth_id, writer_id, counter, envelope) VALUES (?, ?, ?, ?, ?, ?)").bind(user.id).bind(label).bind(birth).bind(&writer).bind(counter).bind(canonical).execute(&db).await.unwrap();
    }
    db.close().await;
    let db = db::db_context::setup_db(&path, &dir.path().join("backups")).await;
    assert_schema(&db).await;
    for (kind, birth) in [("group", &group_birth), ("model", &model_birth)] {
        assert_eq!(
            initialization(&db, user.id, kind, birth).await,
            Some(("unknown".into(), None, None)),
            "legacy record must not prove initialization ancestry"
        );
    }
}

#[tokio::test]
async fn rekey_marks_creation_identity_unknown_but_preserves_original_evidence() {
    let (_dir, db) = common::test_db().await;
    assert_schema(&db).await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, "init-rekey", "stl", 1, None)
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
    let group_creation =
        causal_envelope::get_local_creation_record(&db, &user, EntityKind::Group, &group_birth)
            .await
            .unwrap()
            .unwrap()
            .envelope;
    let model_creation =
        causal_envelope::get_local_creation_record(&db, &user, EntityKind::Model, &model_birth)
            .await
            .unwrap()
            .unwrap()
            .envelope;
    let group_before = initialization(&db, user.id, "group", &group_birth).await;
    let model_before = initialization(&db, user.id, "model", &model_birth).await;
    group_db::edit_group(
        &db,
        &user,
        group.id,
        "folder",
        None,
        Some(&group.unique_global_id),
    )
    .await
    .unwrap();
    rekey_model(&db, &user, model, &model_global).await;
    assert_eq!(
        initialization(&db, user.id, "group", &group_birth).await,
        group_before
    );
    assert_eq!(
        initialization(&db, user.id, "model", &model_birth).await,
        model_before
    );
    let new_group = random_hex_32();
    let new_model = random_hex_32();
    group_db::edit_group(&db, &user, group.id, "folder", None, Some(&new_group))
        .await
        .unwrap();
    rekey_model(&db, &user, model, &new_model).await;
    assert_eq!(
        initialization(&db, user.id, "group", &group_birth).await,
        Some(("unknown".into(), None, None))
    );
    assert_eq!(
        initialization(&db, user.id, "model", &model_birth).await,
        Some(("unknown".into(), None, None))
    );
    assert_eq!(
        causal_envelope::get_local_creation_record(&db, &user, EntityKind::Group, &group_birth)
            .await
            .unwrap()
            .unwrap()
            .envelope,
        group_creation
    );
    assert_eq!(
        causal_envelope::get_local_creation_record(&db, &user, EntityKind::Model, &model_birth)
            .await
            .unwrap()
            .unwrap()
            .envelope,
        model_creation
    );
    for (kind, birth, creation) in [
        ("group", &group_birth, group_creation),
        ("model", &model_birth, model_creation),
    ] {
        let result = sqlx::query("UPDATE entity_births SET initialization_status = 'created', initialization_writer_id = ?, initialization_counter = ? WHERE user_id = ? AND entity_kind = ? AND birth_id = ?")
            .bind(creation.event.writer_id).bind(creation.event.counter.parse::<i64>().unwrap()).bind(user.id).bind(kind).bind(birth).execute(&db).await;
        assert!(
            result.is_err(),
            "old creation evidence must not initialize a rekeyed entity"
        );
    }
}

#[tokio::test]
async fn failed_rekey_invalidation_rolls_back_global_id_and_initialization() {
    let (_dir, db) = common::test_db().await;
    assert_schema(&db).await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, "init-rekey-fail", "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "part", blob, None, None)
        .await
        .unwrap();
    let gb: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let (mg, mb): (String, String) = sqlx::query_as(
        "SELECT model_unique_global_id, model_birth_id FROM models WHERE model_id = ?",
    )
    .bind(model)
    .fetch_one(&db)
    .await
    .unwrap();
    let group_before = initialization(&db, user.id, "group", &gb).await;
    let model_before = initialization(&db, user.id, "model", &mb).await;
    sqlx::query("CREATE TRIGGER reject_rekey_invalidation BEFORE UPDATE OF initialization_status ON entity_births WHEN OLD.initialization_status = 'created' AND NEW.initialization_status = 'unknown' BEGIN SELECT RAISE(ABORT, 'injected failure'); END").execute(&db).await.unwrap();
    assert!(
        group_db::edit_group(&db, &user, group.id, "folder", None, Some(&random_hex_32()))
            .await
            .is_err()
    );
    assert!(
        model_db::edit_model(
            &db,
            &user,
            model,
            "part",
            None,
            None,
            ModelFlags::empty(),
            None,
            Some(&random_hex_32())
        )
        .await
        .is_err()
    );
    let current_group: String =
        sqlx::query_scalar("SELECT group_unique_global_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let current_model: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(model)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(current_group, group.unique_global_id);
    assert_eq!(current_model, mg);
    assert_eq!(
        initialization(&db, user.id, "group", &gb).await,
        group_before
    );
    assert_eq!(
        initialization(&db, user.id, "model", &mb).await,
        model_before
    );
}
