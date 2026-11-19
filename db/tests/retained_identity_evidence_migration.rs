//! Migration attaches a portable birth only when old local evidence proves it.
use db::{
    blob_db, group_db,
    model::{model_group::ModelGroupMeta, user::User},
    random_hex_32,
};

// Construct evidence with the schema available before the retained-identity migration.
async fn historical_group(db: &sqlx::SqlitePool, user: &User, name: &str) -> ModelGroupMeta {
    let birth = random_hex_32();
    let global = random_hex_32();
    let writer = random_hex_32();
    let timestamp = "2020-01-01T00:00:00Z";
    let id = sqlx::query("INSERT INTO models_group (group_name,group_created,group_user_id,group_last_modified,group_unique_global_id,group_birth_id) VALUES (?,?,?,?,?,?)")
        .bind(name).bind(timestamp).bind(user.id).bind(timestamp).bind(&global).bind(&birth).execute(db).await.unwrap().last_insert_rowid();
    let event = db::causal_envelope::CausalEnvelope {
        version: 2,
        entity: db::causal_envelope::Entity {
            kind: db::causal_envelope::EntityKind::Group,
            global_id: global.clone(),
            birth_id: Some(birth.clone()),
        },
        event: db::causal_envelope::Dot {
            writer_id: writer.clone(),
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![],
        action: db::causal_envelope::Action::GroupCreated { name: name.into() },
    };
    sqlx::query("INSERT INTO local_creation_records (user_id,entity_kind,birth_id,writer_id,counter,envelope) VALUES (?,'group',?,?,1,?)")
        .bind(user.id).bind(&birth).bind(&writer).bind(db::causal_envelope::canonical_causal_envelope(&event).unwrap()).execute(db).await.unwrap();
    sqlx::query("UPDATE entity_births SET initialization_status='created', initialization_writer_id=?, initialization_counter=1 WHERE user_id=? AND entity_kind='group' AND birth_id=?")
        .bind(&writer).bind(user.id).bind(&birth).execute(db).await.unwrap();
    ModelGroupMeta {
        id,
        name: name.into(),
        created: timestamp.into(),
        last_modified: timestamp.into(),
        resource_id: None,
        unique_global_id: global,
    }
}

async fn columns(db: &sqlx::SqlitePool, table: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT name FROM pragma_table_info(?)")
        .bind(table)
        .fetch_all(db)
        .await
        .unwrap()
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn migration_backfills_only_unambiguous_live_identity_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|item| item.version < 20_261_113_090_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let foreign_id = db::user_db::add_user(
        &db,
        "other",
        "migration-foreign@example.test",
        &random_hex_32(),
    )
    .await
    .unwrap();
    let known = historical_group(&db, &user, "known").await;
    let purged = historical_group(&db, &user, "purged").await;
    let reused = historical_group(&db, &user, "reused").await;
    let known_birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(known.id)
            .fetch_one(&db)
            .await
            .unwrap();
    sqlx::query("DELETE FROM models_group WHERE group_id IN (?, ?)")
        .bind(purged.id)
        .bind(reused.id)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id, group_birth_id) VALUES ('reused', '2020-01-01T00:00:00Z', ?, '2020-01-01T00:00:00Z', ?, ?)").bind(user.id).bind(&reused.unique_global_id).bind(random_hex_32()).execute(&db).await.unwrap();
    let legacy_global = random_hex_32();
    sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id) VALUES ('legacy', '2020-01-01T00:00:00Z', ?, '2020-01-01T00:00:00Z', ?)").bind(user.id).bind(&legacy_global).execute(&db).await.unwrap();
    let blob = blob_db::add_blob(&db, "migration-model", "stl", 1, None)
        .await
        .unwrap();
    let model = sqlx::query("INSERT INTO models (model_name,model_blob_id,model_added,model_user_id,model_unique_global_id,model_last_modified,model_birth_id) VALUES ('live',?,'2020-01-01T00:00:00Z',?,?,'2020-01-01T00:00:00Z',?)").bind(blob).bind(user.id).bind(random_hex_32()).bind(random_hex_32()).execute(&db).await.unwrap().last_insert_rowid();
    let removed = sqlx::query("INSERT INTO models (model_name,model_blob_id,model_added,model_user_id,model_unique_global_id,model_last_modified,model_birth_id) VALUES ('removed',?,'2020-01-01T00:00:00Z',?,?,'2020-01-01T00:00:00Z',?)").bind(blob).bind(user.id).bind(random_hex_32()).bind(random_hex_32()).execute(&db).await.unwrap().last_insert_rowid();
    let (global, incarnation, birth): (String, String, String) = sqlx::query_as("SELECT model_unique_global_id, model_incarnation, model_birth_id FROM models WHERE model_id = ?").bind(model).fetch_one(&db).await.unwrap();
    let (removed_global, removed_incarnation): (String, String) = sqlx::query_as(
        "SELECT model_unique_global_id, model_incarnation FROM models WHERE model_id = ?",
    )
    .bind(removed)
    .fetch_one(&db)
    .await
    .unwrap();
    let wrong_incarnation = random_hex_32();
    let writer = random_hex_32();
    for (owner, event_global, inc, counter) in [
        (user.id, &global, &incarnation, 1_i64),
        (user.id, &global, &wrong_incarnation, 2),
        (user.id, &removed_global, &removed_incarnation, 3),
        (foreign_id, &global, &incarnation, 4),
    ] {
        sqlx::query("INSERT INTO membership_causal_events (user_id, model_global_id, model_incarnation, replica_id, counter, event_kind, group_global_id, deletion_operation_id) VALUES (?, ?, ?, ?, ?, 'move', '', '')")
            .bind(owner).bind(event_global).bind(inc).bind(&writer).bind(counter).execute(&db).await.unwrap();
    }
    sqlx::query("DELETE FROM models WHERE model_id = ?")
        .bind(removed)
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    let upgraded = db::db_context::setup_db(&path, &dir.path().join("backups")).await;
    assert!(
        columns(&upgraded, "group_sync_heads")
            .await
            .iter()
            .any(|column| column == "identity_birth_id"),
        "missing durable group identity evidence"
    );
    assert!(
        columns(&upgraded, "membership_causal_events")
            .await
            .iter()
            .any(|column| column == "model_birth_id"),
        "missing durable membership identity evidence"
    );
    let head_birth = |global: String| {
        let pool = upgraded.clone();
        async move {
            sqlx::query_scalar::<_, Option<String>>("SELECT identity_birth_id FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?").bind(user.id).bind(global).fetch_one(&pool).await.unwrap()
        }
    };
    assert_eq!(head_birth(known.unique_global_id).await, Some(known_birth));
    assert_eq!(head_birth(purged.unique_global_id).await, None);
    assert_eq!(head_birth(reused.unique_global_id).await, None);
    assert_eq!(head_birth(legacy_global).await, None);
    for (counter, expected) in [(1_i64, Some(birth)), (2, None), (3, None), (4, None)] {
        let actual: Option<String> = sqlx::query_scalar("SELECT model_birth_id FROM membership_causal_events WHERE replica_id = ? AND counter = ?").bind(&writer).bind(counter).fetch_one(&upgraded).await.unwrap();
        assert_eq!(
            actual, expected,
            "wrong evidence backfilled for event {counter}"
        );
    }
}

#[tokio::test]
async fn new_head_and_membership_capture_birth_and_keep_it_after_purge() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    assert!(
        columns(&db, "group_sync_heads")
            .await
            .iter()
            .any(|column| column == "identity_birth_id")
    );
    assert!(
        columns(&db, "membership_causal_events")
            .await
            .iter()
            .any(|column| column == "model_birth_id")
    );
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "known", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, "new-evidence", "stl", 1, None)
        .await
        .unwrap();
    let model = db::model_db::add_model(&db, &user, "known", blob, None, None)
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
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    sqlx::query("DELETE FROM models WHERE model_id = ?")
        .bind(model)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM models_group WHERE group_id = ?")
        .bind(group.id)
        .execute(&db)
        .await
        .unwrap();
    let head: Option<String> = sqlx::query_scalar(
        "SELECT identity_birth_id FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&group.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    let membership: Option<String> = sqlx::query_scalar("SELECT model_birth_id FROM membership_causal_events WHERE user_id = ? AND model_global_id = ? LIMIT 1").bind(user.id).bind(model_global).fetch_one(&db).await.unwrap();
    assert_eq!(head, Some(group_birth));
    assert_eq!(membership, Some(model_birth));
}

#[tokio::test]
async fn same_global_new_birth_is_unknown_and_failed_reuse_rolls_back() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    assert!(
        columns(&db, "group_sync_heads")
            .await
            .iter()
            .any(|column| column == "identity_birth_id")
    );
    let user = User::default();
    let original = group_db::add_empty_group(&db, &user, "old", None)
        .await
        .unwrap();
    let birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(original.id)
            .fetch_one(&db)
            .await
            .unwrap();
    sqlx::query("DELETE FROM models_group WHERE group_id = ?")
        .bind(original.id)
        .execute(&db)
        .await
        .unwrap();
    let before: Option<String> = sqlx::query_scalar(
        "SELECT identity_birth_id FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&original.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(before.as_deref(), Some(birth.as_str()));
    sqlx::query("CREATE TRIGGER reject_head_evidence BEFORE UPDATE OF identity_birth_id ON group_sync_heads BEGIN SELECT RAISE(ABORT, 'injected failure'); END").execute(&db).await.unwrap();
    let replacement_birth = random_hex_32();
    let insert = sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id, group_birth_id) VALUES ('new', '2020-01-01T00:00:00Z', ?, '2020-01-01T00:00:00Z', ?, ?)")
        .bind(user.id).bind(&original.unique_global_id).bind(&replacement_birth).execute(&db).await;
    assert!(insert.is_err());
    let current: Option<String> = sqlx::query_scalar(
        "SELECT identity_birth_id FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&original.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(current, before);
    let registry: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entity_births WHERE user_id = ? AND entity_kind = 'group' AND birth_id = ?").bind(user.id).bind(&replacement_birth).fetch_one(&db).await.unwrap();
    assert_eq!(registry, 0);
    sqlx::query("DROP TRIGGER reject_head_evidence")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id, group_birth_id) VALUES ('new', '2020-01-01T00:00:00Z', ?, '2020-01-01T00:00:00Z', ?, ?)")
        .bind(user.id).bind(&original.unique_global_id).bind(replacement_birth).execute(&db).await.unwrap();
    let after: Option<String> = sqlx::query_scalar(
        "SELECT identity_birth_id FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&original.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
        after, None,
        "reusing a global id with a new birth is not proof of continuity"
    );
}
