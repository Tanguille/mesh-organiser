//! Integration tests for recovering from cross-platform migration checksums.

use std::{panic::AssertUnwindSafe, path::Path};

use db::db_context;
use futures::FutureExt;
use sqlx::Row;
use tempfile::tempdir;

/// Rewrites every stored checksum to the one a CRLF checkout would have
/// produced, mimicking a database last migrated on Windows.
async fn rewrite_checksums_as_crlf(db_path: &Path) {
    let db = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display()))
        .await
        .unwrap();

    for migration in sqlx::migrate!("./migrations").iter() {
        let crlf = migration
            .sql
            .as_str()
            .replace("\r\n", "\n")
            .replace('\n', "\r\n");
        let checksum = <sha2::Sha384 as sha2::Digest>::digest(crlf.as_bytes()).to_vec();

        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
            .bind(checksum)
            .bind(migration.version)
            .execute(&db)
            .await
            .unwrap();
    }

    db.close().await;
}

#[tokio::test]
async fn setup_db_recovers_from_foreign_line_ending_checksums() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("db.sqlite");
    let backup_dir = dir.path().join("backups");

    let db = db_context::setup_db(&db_path, &backup_dir).await;
    db.close().await;

    rewrite_checksums_as_crlf(&db_path).await;

    // Without the repair this panics with sqlx's VersionMismatch.
    let db = db_context::setup_db(&db_path, &backup_dir).await;

    let rows = sqlx::query("SELECT version, checksum FROM _sqlx_migrations")
        .fetch_all(&db)
        .await
        .unwrap();

    for migration in sqlx::migrate!("./migrations").iter() {
        let row = rows
            .iter()
            .find(|row| row.get::<i64, _>("version") == migration.version)
            .expect("migration should still be recorded as applied");

        assert_eq!(
            row.get::<Vec<u8>, _>("checksum"),
            migration.checksum.as_ref(),
            "checksum for {} should be rewritten to the local one",
            migration.version
        );
    }
}

#[tokio::test]
async fn setup_db_leaves_genuinely_modified_migrations_failing() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("db.sqlite");
    let backup_dir = dir.path().join("backups");

    let db = db_context::setup_db(&db_path, &backup_dir).await;
    db.close().await;

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db_path.display()))
        .await
        .unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = (SELECT MIN(version) FROM _sqlx_migrations)")
        .bind(vec![0u8; 48])
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let result = AssertUnwindSafe(db_context::setup_db(&db_path, &backup_dir))
        .catch_unwind()
        .await;

    assert!(
        result.is_err(),
        "a migration edited beyond line endings must still abort startup"
    );
}
