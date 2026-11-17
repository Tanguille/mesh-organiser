//! Historical deletion heads survive recovery-snapshot expiration.
mod common;
use db::{model::user::User, random_hex_32};

#[tokio::test]
async fn migration_backfills_unresolved_deletion_for_a_purged_group() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version < 20_261_112_180_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let group = common::legacy_group(&db, &user, "old").await;
    let old_id = random_hex_32();
    let current_id = random_hex_32();
    for (operation_id, restored) in [(&old_id, Some("2020-01-01T00:00:00Z")), (&current_id, None)] {
        sqlx::query("INSERT INTO group_deletion_records (user_id, group_global_id, operation_id, deleted_at, restored_at, group_revision) VALUES (?, ?, ?, '2020-01-01T00:00:00Z', ?, 0)")
            .bind(user.id).bind(&group.unique_global_id).bind(operation_id).bind(restored).execute(&db).await.unwrap();
    }
    sqlx::query("DELETE FROM models_group WHERE group_id = ?")
        .bind(group.id)
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    let upgraded = db::db_context::setup_db(&path, &dir.path().join("backups")).await;
    let event: Option<String> = sqlx::query_scalar("SELECT last_deletion_operation_id FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?")
        .bind(user.id).bind(group.unique_global_id).fetch_one(&upgraded).await.unwrap();
    assert_eq!(event.as_deref(), Some(current_id.as_str()));
}
