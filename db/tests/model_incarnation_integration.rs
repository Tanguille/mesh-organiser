//! Model incarnation tokens survive upgrades and prevent row identity reuse.

use db::{blob_db, db_context::DbContext, model::user::User};

mod common;

async fn add_model(db: &DbContext, name: &str) -> i64 {
    let blob = blob_db::add_blob(db, name, "stl", 1, None).await.unwrap();
    db::model_db::add_model(db, &User::default(), name, blob, None, None)
        .await
        .unwrap()
}

async fn incarnation(db: &DbContext, id: i64) -> String {
    sqlx::query_scalar("SELECT model_incarnation FROM models WHERE model_id = ?")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn migration_backfills_existing_models_with_stable_distinct_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(dir.path().join("db.sqlite"))
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let migrations = sqlx::migrate!("./migrations")
        .iter()
        .filter(|migration| migration.version < 20_261_112_170_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(migrations)
        .run(&db)
        .await
        .unwrap();
    let first = common::legacy_model(&db, &User::default(), "legacy-first").await;
    let second = common::legacy_model(&db, &User::default(), "legacy-second").await;
    db.close().await;
    let backup = dir.path().join("backups");
    std::fs::create_dir_all(&backup).unwrap();
    let db = db::db_context::setup_db(&dir.path().join("db.sqlite"), &backup).await;
    let token_first = incarnation(&db, first).await;
    let token_second = incarnation(&db, second).await;
    assert_ne!(token_first, "");
    assert_ne!(token_second, "");
    assert_ne!(token_first, token_second);
    db.close().await;
    let db = db::db_context::setup_db(&dir.path().join("db.sqlite"), &backup).await;
    assert_eq!(incarnation(&db, first).await, token_first);
    assert_eq!(incarnation(&db, second).await, token_second);
}

#[tokio::test]
async fn model_incarnation_cannot_be_replaced_or_cleared() {
    let (_dir, db) = common::test_db().await;
    let id = add_model(&db, "immutable").await;
    let original = incarnation(&db, id).await;
    assert_ne!(original, "");
    for replacement in ["", "invalid", "00000000000000000000000000000000"] {
        assert!(
            sqlx::query("UPDATE models SET model_incarnation = ? WHERE model_id = ?")
                .bind(replacement)
                .bind(id)
                .execute(&db)
                .await
                .is_err()
        );
        assert_eq!(incarnation(&db, id).await, original);
    }
}

#[tokio::test]
async fn explicit_invalid_incarnation_insert_fails_atomically() {
    let (_dir, db) = common::test_db().await;
    let blob = blob_db::add_blob(&db, "invalid-token-model", "stl", 1, None)
        .await
        .unwrap();
    let result = sqlx::query("INSERT INTO models (model_name, model_blob_id, model_added, model_user_id, model_unique_global_id, model_last_modified, model_incarnation) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind("invalid-token-model").bind(blob).bind("2026-01-01T00:00:00Z").bind(User::default().id)
        .bind(db::random_hex_32()).bind("2026-01-01T00:00:00Z").bind("invalid")
        .execute(&db).await;
    assert!(result.is_err());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models WHERE model_name = ?")
        .bind("invalid-token-model")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
