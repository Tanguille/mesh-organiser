//! Durable group heads make stale sync baselines distinguishable across lifecycles.
mod common;

use db::{
    blob_db, db_context::DbContext, group_db, model::user::User, model_db, random_hex_32, user_db,
};

async fn head(db: &DbContext, user: &User, global: &str) -> (i64, i64, String) {
    let snapshot = group_db::get_group_sync_snapshot(db, user).await.unwrap();
    let entry = snapshot.heads.iter().find(|h| h.group_global_id == global);
    assert!(entry.is_some(), "missing durable head for {global}");
    let entry = entry.unwrap();
    (entry.generation, entry.head, entry.state.clone())
}

async fn add_model(db: &DbContext, user: &User) -> i64 {
    let blob = blob_db::add_blob(db, "part", "stl", 1, None).await.unwrap();
    model_db::add_model(db, user, "part", blob, None, None)
        .await
        .unwrap()
}

#[tokio::test]
async fn lifecycle_heads_survive_purge_and_same_id_recreation() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let global = &group.unique_global_id;
    let created = head(&db, &user, global).await;
    assert!(created.0 > 0 && created.1 > 0);
    assert_eq!(created.2, "active");
    let model = add_model(&db, &user).await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    let assigned = head(&db, &user, global).await;
    assert_eq!(assigned.0, created.0);
    assert!(assigned.1 > created.1);
    model_db::trash_models(&db, &user, &[model]).await.unwrap();
    let visibility = head(&db, &user, global).await;
    assert!(visibility.1 > assigned.1);
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let deleted = head(&db, &user, global).await;
    assert_eq!(deleted.2, "trashed");
    assert!(deleted.1 > visibility.1);
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    assert_eq!(head(&db, &user, global).await, deleted);
    let restore = common::group_restore_request(&db, &user, group.id).await;
    group_db::restore_groups(&db, &user, std::slice::from_ref(&restore), 30)
        .await
        .unwrap();
    let restored = head(&db, &user, global).await;
    assert_eq!(restored.2, "active");
    assert_eq!(restored.0, created.0);
    assert!(restored.1 > deleted.1);
    assert_ne!(
        restored, created,
        "a stale pre-deletion baseline must not match after restoration"
    );
    group_db::restore_groups(&db, &user, &[restore], 30)
        .await
        .unwrap();
    assert_eq!(head(&db, &user, global).await, restored);
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(&db)
    .await
    .unwrap();
    let before_purge = head(&db, &user, global).await;
    group_db::purge_trashed_groups(&db, 30).await.unwrap();
    let purged = head(&db, &user, global).await;
    assert_eq!(purged.2, "absent");
    assert!(purged.1 > before_purge.1);
    // Exercise the database head guard directly; the public API rejects this rekey.
    sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id) VALUES ('replacement', '2099-01-01T00:00:00Z', ?, '2099-01-01T00:00:00Z', ?)")
        .bind(user.id)
        .bind(global)
        .execute(&db)
        .await
        .unwrap();
    let recreated = head(&db, &user, global).await;
    assert_eq!(recreated.2, "active");
    assert!(recreated.0 > created.0);
    assert!(recreated.1 > purged.1);
    assert_ne!((recreated.0, recreated.1), (created.0, created.1));
}

#[tokio::test]
async fn rekey_retires_old_identity_and_heads_are_owner_scoped() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let foreign_id = user_db::add_user(&db, "foreign", "foreign@example.test", &random_hex_32())
        .await
        .unwrap();
    let foreign = user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let own = group_db::add_empty_group(&db, &user, "own", None)
        .await
        .unwrap();
    let other = group_db::add_empty_group(&db, &foreign, "foreign", None)
        .await
        .unwrap();
    let original = own.unique_global_id.clone();
    group_db::edit_group(&db, &foreign, other.id, "foreign", None, Some(&original))
        .await
        .unwrap();
    let foreign_before = head(&db, &foreign, &original).await;
    let before = head(&db, &user, &original).await;
    let next = random_hex_32();
    group_db::edit_group(&db, &user, own.id, "own", None, Some(&next))
        .await
        .unwrap();
    let retired = head(&db, &user, &original).await;
    assert_eq!(retired.2, "absent");
    assert!(retired.1 > before.1);
    let new_head = head(&db, &user, &next).await;
    assert_eq!(new_head.2, "active");
    assert_eq!(head(&db, &foreign, &original).await, foreign_before);
    group_db::edit_group(&db, &user, own.id, "own", None, Some(&original))
        .await
        .unwrap();
    let reused = head(&db, &user, &original).await;
    assert_eq!(reused.2, "active");
    assert!(reused.0 > before.0 && reused.1 > retired.1);
    assert_eq!(head(&db, &user, &next).await.2, "absent");
    assert_eq!(head(&db, &foreign, &original).await, foreign_before);
}

#[tokio::test]
async fn failed_head_update_rolls_back_group_mutation() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "before", None)
        .await
        .unwrap();
    let old_head = head(&db, &user, &group.unique_global_id).await;
    sqlx::query("CREATE TRIGGER reject_group_head BEFORE UPDATE ON group_sync_heads BEGIN SELECT RAISE(ABORT, 'head update failure'); END").execute(&db).await.unwrap();
    assert!(
        group_db::edit_group(&db, &user, group.id, "after", None, None)
            .await
            .is_err()
    );
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id = ?")
        .bind(group.id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(name, "before");
    assert_eq!(head(&db, &user, &group.unique_global_id).await, old_head);
}

async fn pre_heads_database() -> (tempfile::TempDir, DbContext) {
    let dir = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(dir.path().join("db.sqlite"))
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let migrations = sqlx::migrate!("./migrations")
        .iter()
        .filter(|migration| migration.version < 20_261_112_160_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(migrations)
        .run(&db)
        .await
        .unwrap();
    (dir, db)
}

// Historical fixtures predate the current delete API and its new schema columns.
async fn historical_delete(db: &DbContext, user: &User, id: i64) {
    sqlx::query("UPDATE models_group SET group_deleted_at = '2099-01-01T00:00:00Z' WHERE group_id = ? AND group_user_id = ?")
        .bind(id).bind(user.id).execute(db).await.unwrap();
    sqlx::query("INSERT INTO group_deletion_records (user_id, group_global_id, operation_id, deleted_at, group_revision) SELECT group_user_id, group_unique_global_id, ?, group_deleted_at, group_revision FROM models_group WHERE group_id = ? AND group_user_id = ?")
        .bind(random_hex_32()).bind(id).bind(user.id).execute(db).await.unwrap();
}

#[tokio::test]
async fn upgrade_backfills_active_trashed_and_purged_identities() {
    let (dir, db) = pre_heads_database().await;
    let user = User::default();
    let active = common::legacy_group(&db, &user, "active").await;
    let trashed = common::legacy_group(&db, &user, "trashed").await;
    let purged = common::legacy_group(&db, &user, "purged").await;
    historical_delete(&db, &user, trashed.id).await;
    historical_delete(&db, &user, purged.id).await;
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(purged.id)
    .execute(&db)
    .await
    .unwrap();
    // Emulate the schema-era purge while keeping its durable deletion record.
    let result = sqlx::query(
        "DELETE FROM models_group WHERE group_id = ? AND group_deleted_at < '2001-01-01T00:00:00Z'",
    )
    .bind(purged.id)
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(result.rows_affected(), 1);
    db.close().await;
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    for (global, state) in [
        (&active.unique_global_id, "active"),
        (&trashed.unique_global_id, "trashed"),
        (&purged.unique_global_id, "absent"),
    ] {
        let current = head(&db, &user, global).await;
        assert_eq!(current.2, state);
        assert!(current.0 > 0 && current.1 > 0);
    }
}

#[tokio::test]
async fn upgrade_allows_historical_duplicate_ids_but_snapshot_rejects_ambiguity() {
    let (dir, db) = pre_heads_database().await;
    let user = User::default();
    let first = common::legacy_group(&db, &user, "first").await;
    let second = common::legacy_group(&db, &user, "second").await;
    sqlx::query("UPDATE models_group SET group_unique_global_id = ? WHERE group_id = ?")
        .bind(&first.unique_global_id)
        .bind(second.id)
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    let upgraded =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    assert!(
        group_db::get_group_sync_snapshot(&upgraded, &user)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn resolving_historical_duplicate_by_rekey_preserves_remaining_trashed_head() {
    let (dir, db) = pre_heads_database().await;
    let user = User::default();
    let first = common::legacy_group(&db, &user, "trashed").await;
    let second = common::legacy_group(&db, &user, "active").await;
    historical_delete(&db, &user, first.id).await;
    sqlx::query("UPDATE models_group SET group_unique_global_id = ? WHERE group_id = ?")
        .bind(&first.unique_global_id)
        .bind(second.id)
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    assert!(group_db::get_group_sync_snapshot(&db, &user).await.is_err());
    group_db::edit_group(
        &db,
        &user,
        second.id,
        "active",
        None,
        Some(&random_hex_32()),
    )
    .await
    .unwrap();
    assert_eq!(head(&db, &user, &first.unique_global_id).await.2, "trashed");
}

#[tokio::test]
async fn purging_historical_duplicate_preserves_remaining_active_head() {
    let (dir, db) = pre_heads_database().await;
    let user = User::default();
    let first = common::legacy_group(&db, &user, "active").await;
    let second = common::legacy_group(&db, &user, "trashed").await;
    historical_delete(&db, &user, second.id).await;
    sqlx::query("UPDATE models_group SET group_unique_global_id = ?, group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?")
        .bind(&first.unique_global_id).bind(second.id).execute(&db).await.unwrap();
    db.close().await;
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    assert!(group_db::get_group_sync_snapshot(&db, &user).await.is_err());
    group_db::purge_trashed_groups(&db, 30).await.unwrap();
    assert_eq!(head(&db, &user, &first.unique_global_id).await.2, "active");
}
