//! Stale numeric selections must not target newly created models or projects.
mod common;
use db::{
    blob_db, group_db,
    model::{ModelFlags, resource::ResourceFlags, user::User},
    model_db, random_hex_32, resource_db,
};

#[tokio::test]
async fn purged_model_id_is_not_reused_and_stale_actions_leave_replacement_unchanged() {
    let (dir, db) = common::test_db().await;
    let user = User::default();
    let blob = blob_db::add_blob(&db, "old-model", "stl", 1, None)
        .await
        .unwrap();
    let old = model_db::add_model(&db, &user, "old", blob, None, None)
        .await
        .unwrap();
    model_db::trash_models(&db, &user, &[old]).await.unwrap();
    sqlx::query("UPDATE models SET model_deleted = '2000-01-01T00:00:00Z' WHERE model_id = ?")
        .bind(old)
        .execute(&db)
        .await
        .unwrap();
    model_db::purge_trashed_models(&db, 30).await.unwrap();
    db.close().await;
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    let new = model_db::add_model(&db, &user, "replacement", blob, None, None)
        .await
        .unwrap();
    assert!(new > old, "a purged model id must not be reused");
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let _ = model_db::edit_model(
        &db,
        &user,
        old,
        "wrong",
        None,
        None,
        ModelFlags::empty(),
        None,
        None,
    )
    .await;
    let _ = model_db::trash_models(&db, &user, &[old]).await;
    let _ = group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![old], None).await;
    let replacement: (String, Option<String>, Option<i64>) = sqlx::query_as(
        "SELECT model_name, model_deleted, model_group_id FROM models WHERE model_id = ?",
    )
    .bind(new)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(replacement, ("replacement".into(), None, None));
}

#[tokio::test]
async fn deleted_resource_id_is_not_reused_and_stale_actions_cannot_touch_replacement() {
    let (dir, db) = common::test_db().await;
    let user = User::default();
    let old = resource_db::add_resource(&db, &user, "old", None)
        .await
        .unwrap();
    resource_db::delete_resource(&db, &user, old.id)
        .await
        .unwrap();
    db.close().await;
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    let new = resource_db::add_resource(&db, &user, "replacement", None)
        .await
        .unwrap();
    assert!(new.id > old.id, "a deleted project id must not be reused");
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, "project-model", "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "part", blob, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    resource_db::set_resource_on_group(&db, &user, Some(new.id), group.id, None)
        .await
        .unwrap();
    let _ = resource_db::edit_resource(
        &db,
        &user,
        old.id,
        "wrong",
        ResourceFlags::empty(),
        None,
        None,
    )
    .await;
    let _ = resource_db::delete_resource(&db, &user, old.id).await;
    let _ = resource_db::set_resource_on_group(&db, &user, Some(old.id), group.id, None).await;
    let name: String =
        sqlx::query_scalar("SELECT resource_name FROM resources WHERE resource_id = ?")
            .bind(new.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT group_resource_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(name, "replacement");
    assert_eq!(assigned, Some(new.id));
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_model_and_resource_creations_have_unique_committed_ids() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let blob = blob_db::add_blob(&db, "concurrent-model", "stl", 1, None)
        .await
        .unwrap();
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let pool = db.clone();
        let user = user.clone();
        tasks.push(tokio::spawn(async move {
            let model = model_db::add_model(&pool, &user, "part", blob, None, None)
                .await
                .unwrap();
            let resource = resource_db::add_resource(&pool, &user, "project", None)
                .await
                .unwrap()
                .id;
            (model, resource)
        }));
    }
    let mut models = Vec::new();
    let mut resources = Vec::new();
    for task in tasks {
        let (model, resource) = task.await.unwrap();
        models.push(model);
        resources.push(resource);
    }
    models.sort_unstable();
    resources.sort_unstable();
    models.dedup();
    resources.dedup();
    assert_eq!(models.len(), 12);
    assert_eq!(resources.len(), 12);
}

#[tokio::test]
async fn raw_high_ids_are_observed_by_future_normal_creates() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let blob = blob_db::add_blob(&db, "raw-model", "stl", 1, None)
        .await
        .unwrap();
    sqlx::query("INSERT INTO models (model_id, model_name, model_blob_id, model_added, model_user_id, model_unique_global_id, model_last_modified) VALUES (5000, 'raw', ?, '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')").bind(blob).bind(user.id).bind(random_hex_32()).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO resources (resource_id, resource_name, resource_created, resource_user_id, resource_unique_global_id, resource_last_modified) VALUES (6000, 'raw', '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')").bind(user.id).bind(random_hex_32()).execute(&db).await.unwrap();
    sqlx::query("DELETE FROM models WHERE model_id = 5000")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM resources WHERE resource_id = 6000")
        .execute(&db)
        .await
        .unwrap();
    let stale_model = sqlx::query("INSERT INTO models (model_id, model_name, model_blob_id, model_added, model_user_id, model_unique_global_id, model_last_modified) VALUES (4999, 'stale', ?, '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')")
        .bind(blob).bind(user.id).bind(random_hex_32()).execute(&db).await;
    let stale_resource = sqlx::query("INSERT INTO resources (resource_id, resource_name, resource_created, resource_user_id, resource_unique_global_id, resource_last_modified) VALUES (5999, 'stale', '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')")
        .bind(user.id).bind(random_hex_32()).execute(&db).await;
    assert!(
        stale_model.is_err(),
        "raw inserts must not revive an earlier model id"
    );
    assert!(
        stale_resource.is_err(),
        "raw inserts must not revive an earlier project id"
    );
    let model = model_db::add_model(&db, &user, "after", blob, None, None)
        .await
        .unwrap();
    let resource = resource_db::add_resource(&db, &user, "after", None)
        .await
        .unwrap();
    assert!(model > 5000);
    assert!(resource.id > 6000);
}

#[tokio::test]
async fn failed_creation_does_not_commit_allocator_state() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let blob = blob_db::add_blob(&db, "failure", "stl", 1, None)
        .await
        .unwrap();
    let before: Vec<(String, i64)> = sqlx::query_as(
        "SELECT object_kind, highwater FROM object_id_highwater ORDER BY object_kind",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER reject_model_creation AFTER INSERT ON models BEGIN SELECT RAISE(FAIL, 'injected failure'); END").execute(&db).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_resource_creation AFTER INSERT ON resources BEGIN SELECT RAISE(FAIL, 'injected failure'); END").execute(&db).await.unwrap();
    assert!(
        model_db::add_model(&db, &user, "failure", blob, None, None)
            .await
            .is_err()
    );
    assert!(
        resource_db::add_resource(&db, &user, "failure", None)
            .await
            .is_err()
    );
    let after: Vec<(String, i64)> = sqlx::query_as(
        "SELECT object_kind, highwater FROM object_id_highwater ORDER BY object_kind",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(after, before);
    let counts: (i64, i64) =
        sqlx::query_as("SELECT (SELECT COUNT(*) FROM models), (SELECT COUNT(*) FROM resources)")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(counts, (0, 0));
}

#[tokio::test]
async fn migration_starts_allocators_above_existing_high_ids() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version < 20_261_113_010_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let blob = blob_db::add_blob(&db, "old-high", "stl", 1, None)
        .await
        .unwrap();
    sqlx::query("INSERT INTO models (model_id, model_name, model_blob_id, model_added, model_user_id, model_unique_global_id, model_last_modified) VALUES (4000, 'legacy', ?, '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')").bind(blob).bind(user.id).bind(random_hex_32()).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO resources (resource_id, resource_name, resource_created, resource_user_id, resource_unique_global_id, resource_last_modified) VALUES (6000, 'legacy', '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')").bind(user.id).bind(random_hex_32()).execute(&db).await.unwrap();
    db.close().await;
    let db = db::db_context::setup_db(&path, &dir.path().join("backups")).await;
    let next_model = model_db::add_model(&db, &user, "after", blob, None, None)
        .await
        .unwrap();
    let next_resource = resource_db::add_resource(&db, &user, "after", None)
        .await
        .unwrap();
    assert!(next_model > 4000);
    assert!(next_resource.id > 6000);
    let violations: Vec<(String, i64, String, i64)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(violations, []);
    // Deleted high ids from before migration cannot be recovered from this schema.
}

#[tokio::test]
async fn sqlite_rowid_aliases_cannot_change_model_or_project_identity() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let blob = blob_db::add_blob(&db, "rowid-model", "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "part", blob, None, None)
        .await
        .unwrap();
    let project = resource_db::add_resource(&db, &user, "project", None)
        .await
        .unwrap();
    let highwater: Vec<(String, i64)> = sqlx::query_as(
        "SELECT object_kind, highwater FROM object_id_highwater ORDER BY object_kind",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    let statements = [
        "UPDATE models SET rowid = 5000 WHERE model_id = ?",
        "UPDATE models SET _rowid_ = 5000 WHERE model_id = ?",
        "UPDATE models SET oid = 5000 WHERE model_id = ?",
        "UPDATE resources SET rowid = 6000 WHERE resource_id = ?",
        "UPDATE resources SET _rowid_ = 6000 WHERE resource_id = ?",
        "UPDATE resources SET oid = 6000 WHERE resource_id = ?",
    ];
    for (index, statement) in statements.into_iter().enumerate() {
        let original = if index < 3 { model } else { project.id };
        let result = sqlx::query(sqlx::AssertSqlSafe(statement))
            .bind(original)
            .execute(&db)
            .await;
        assert!(
            result.is_err(),
            "rowid alias must not change object identity: {statement}"
        );
    }
    let after: Vec<(String, i64)> = sqlx::query_as(
        "SELECT object_kind, highwater FROM object_id_highwater ORDER BY object_kind",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(after, highwater);
    let model_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models WHERE model_id = ?)")
            .bind(model)
            .fetch_one(&db)
            .await
            .unwrap();
    let project_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM resources WHERE resource_id = ?)")
            .bind(project.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(model_exists && project_exists);
}
