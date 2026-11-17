//! Sync identities remain stable across ordinary account and database operations.

mod common;

use db::{
    db_context::{self, DbContext},
    model::user::User,
    random_hex_32, user_db,
};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

async fn assert_identity_schema(db: &DbContext) {
    for table in ["sync_instance", "sync_accounts"] {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
        )
        .bind(table)
        .fetch_one(db)
        .await
        .unwrap();
        assert_eq!(count, 1, "missing sync identity table {table}");
    }
}

async fn instance(db: &DbContext) -> String {
    sqlx::query_scalar("SELECT instance_id FROM sync_instance WHERE singleton = 1")
        .fetch_one(db)
        .await
        .unwrap()
}

async fn account(db: &DbContext, user_id: i64) -> Option<String> {
    sqlx::query_scalar("SELECT account_id FROM sync_accounts WHERE user_id = ?")
        .bind(user_id)
        .fetch_optional(db)
        .await
        .unwrap()
}

fn assert_random_identity(value: &str) {
    assert_eq!(value.len(), 32);
    assert!(
        value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
}

#[tokio::test]
async fn independent_databases_and_accounts_receive_distinct_identities() {
    let (_first_dir, first) = common::test_db().await;
    let (_second_dir, second) = common::test_db().await;
    assert_identity_schema(&first).await;
    assert_identity_schema(&second).await;
    let local_id = User::default().id;
    let first_instance = instance(&first).await;
    let second_instance = instance(&second).await;
    let first_local = account(&first, local_id).await.unwrap();
    let second_local = account(&second, local_id).await.unwrap();
    let other = user_db::add_user(&first, "other", "other@example.com", &random_hex_32())
        .await
        .unwrap();
    let other_account = account(&first, other).await.unwrap();
    for value in [
        &first_instance,
        &second_instance,
        &first_local,
        &second_local,
        &other_account,
    ] {
        assert_random_identity(value);
    }
    assert_ne!(first_instance, second_instance);
    assert_ne!(first_local, second_local);
    assert_ne!(first_local, other_account);
    assert_eq!(account(&first, -1).await, None);
    let local_identity = user_db::get_sync_identity(&first, local_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(local_identity.instance_id, first_instance);
    assert_eq!(local_identity.account_id, first_local);
    let other_identity = user_db::get_sync_identity(&first, other)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(other_identity.instance_id, first_instance);
    assert_eq!(other_identity.account_id, other_account);
    assert!(
        user_db::get_sync_identity(&first, -1)
            .await
            .unwrap()
            .is_none()
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_instance")
        .fetch_one(&first)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn changing_credentials_and_reopening_database_preserves_identities() {
    let (dir, db) = common::test_db().await;
    assert_identity_schema(&db).await;
    let user_id = User::default().id;
    let before = (instance(&db).await, account(&db, user_id).await.unwrap());
    user_db::set_user_sync_token(&db, user_id, "first", "https://example.test", false)
        .await
        .unwrap();
    user_db::set_user_sync_token(&db, user_id, "second", "https://example.test", false)
        .await
        .unwrap();
    user_db::scramble_login_token(&db, user_id).await.unwrap();
    user_db::edit_user_min(&db, user_id, "renamed", "renamed@example.test")
        .await
        .unwrap();
    user_db::clear_user_sync(&db, user_id).await.unwrap();
    assert_eq!(
        before,
        (instance(&db).await, account(&db, user_id).await.unwrap())
    );
    db.close().await;
    // Copies and restored backups retain the stored identity; independent live clones need re-keying.
    let reopened =
        db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    assert_eq!(
        before,
        (
            instance(&reopened).await,
            account(&reopened, user_id).await.unwrap()
        )
    );
}

#[tokio::test]
async fn upgrade_assigns_identities_to_existing_users() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = SqlitePool::connect_with(options).await.unwrap();
    let old_migrations = sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version < 20_261_112_130_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old_migrations)
        .run(&db)
        .await
        .unwrap();
    let other = user_db::add_user(&db, "old-user", "old@example.test", &random_hex_32())
        .await
        .unwrap();
    db.close().await;
    let upgraded = db_context::setup_db(&path, &dir.path().join("backups")).await;
    assert_identity_schema(&upgraded).await;
    let first = account(&upgraded, User::default().id).await.unwrap();
    let second = account(&upgraded, other).await.unwrap();
    assert_random_identity(&first);
    assert_random_identity(&second);
    assert_ne!(first, second);
    let saved_instance = instance(&upgraded).await;
    upgraded.close().await;
    let reopened = db_context::setup_db(&path, &dir.path().join("backups")).await;
    assert_eq!(instance(&reopened).await, saved_instance);
    assert_eq!(account(&reopened, User::default().id).await, Some(first));
    assert_eq!(account(&reopened, other).await, Some(second));
}

#[tokio::test]
async fn identity_creation_failure_rolls_back_user_creation() {
    let (_dir, db) = common::test_db().await;
    assert_identity_schema(&db).await;
    sqlx::query("CREATE TRIGGER reject_sync_identity BEFORE INSERT ON sync_accounts BEGIN SELECT RAISE(ABORT, 'identity failure'); END")
        .execute(&db).await.unwrap();
    let result = user_db::add_user(&db, "broken", "broken@example.test", &random_hex_32()).await;
    assert!(result.is_err());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE user_email = ?")
        .bind("broken@example.test")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0, "a user must not exist without a sync identity");
}
