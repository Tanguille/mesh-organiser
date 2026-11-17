use std::{fs, path::Path};

use sha2::{Digest, Sha384};
use sqlx::{Sqlite, migrate::MigrateDatabase};

pub type DbContext = sqlx::Pool<Sqlite>;

/// Opens or creates a `SQLite` database, backs it up, and applies the embedded migrations.
///
/// Legacy databases are deduplicated when required before migrations run. The returned pool
/// is ready to serve queries.
///
/// # Panics
///
/// Panics if the database or backup cannot be accessed, or if preparation or migration fails.
pub async fn setup_db(sqlite_path: &Path, sqlite_backup_dir: &Path) -> DbContext {
    let url = format!(
        "sqlite:{}",
        sqlite_path.to_str().expect("path should be something")
    );

    if !Sqlite::database_exists(url.as_str()).await.unwrap() {
        Sqlite::create_database(url.as_str())
            .await
            .expect("failed to create database");
    }

    let connection_option = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(sqlite_path)
        .busy_timeout(std::time::Duration::from_secs(15));

    let db = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connection_option)
        .await
        .unwrap();

    let migration_count = get_db_migration_count(&db).await;

    // Snapshot before the dedup delete and the migration run below, so a failure
    // partway through leaves a recoverable copy of the pre-migration database.
    // VACUUM INTO includes committed rows still present in SQLite's WAL.
    backup_db(&db, sqlite_path, sqlite_backup_dir).await;

    dedupe_models_before_blob_migration(&db, migration_count)
        .await
        .expect("failed to dedupe models before blob migration");

    let migrator = sqlx::migrate!("./migrations");
    repair_line_ending_checksums(&db, &migrator).await;
    migrator
        .run(&db)
        .await
        .expect("failed to run database migrations");

    let new_migration_count = get_db_migration_count(&db).await;

    if new_migration_count > migration_count {
        sqlx::query!("VACUUM")
            .execute(&db)
            .await
            .expect("Failed to vacuum database after migrations");
    }

    // A reopened backup must not issue events under its former writer counter.
    let mut transaction = db
        .begin_with("BEGIN IMMEDIATE")
        .await
        .expect("failed to start writer rotation");
    sqlx::query("INSERT OR IGNORE INTO local_causal_writers (replica_id) SELECT replica_id FROM group_causal_replica WHERE singleton = 1")
        .execute(&mut *transaction)
        .await
        .expect("failed to preserve causal writer");
    // A missing singleton row is tolerated: the register below then inserts nothing, and the
    // library must still open rather than abort startup.
    sqlx::query("UPDATE group_causal_replica SET replica_id = ?, counter = 0 WHERE singleton = 1")
        .bind(crate::random_hex_32())
        .execute(&mut *transaction)
        .await
        .expect("failed to rotate causal writer");
    sqlx::query("INSERT INTO local_causal_writers (replica_id) SELECT replica_id FROM group_causal_replica WHERE singleton = 1")
        .execute(&mut *transaction)
        .await
        .expect("failed to register causal writer");
    crate::sync_transport::rotate_all(&mut transaction)
        .await
        .expect("failed to rotate account writers");
    transaction
        .commit()
        .await
        .expect("failed to commit writer rotation");

    db
}

/// Rewrites stored migration checksums that differ from the embedded migrations
/// only in line endings.
///
/// sqlx hashes the raw bytes of each migration file, so a CRLF checkout and an
/// LF one disagree about byte-identical SQL — and the database can be shared
/// between platforms through a network data path. Checksums matching neither
/// variant are left alone, so a genuinely modified migration still fails.
async fn repair_line_ending_checksums(db: &DbContext, migrator: &sqlx::migrate::Migrator) {
    let applied: Vec<(i64, Vec<u8>)> =
        match sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations")
            .fetch_all(db)
            .await
        {
            Ok(rows) => rows,
            // No migration table yet: nothing has been applied to repair.
            Err(_) => return,
        };

    for migration in migrator.iter() {
        let Some((_, stored)) = applied
            .iter()
            .find(|(version, _)| *version == migration.version)
        else {
            continue;
        };

        if stored.as_slice() == migration.checksum.as_ref() {
            continue;
        }

        let lf = migration.sql.as_str().replace("\r\n", "\n");
        let crlf = lf.replace('\n', "\r\n");

        if stored.as_slice() != Sha384::digest(lf.as_bytes()).as_slice()
            && stored.as_slice() != Sha384::digest(crlf.as_bytes()).as_slice()
        {
            continue;
        }

        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
            .bind(migration.checksum.as_ref())
            .bind(migration.version)
            .execute(db)
            .await
            .expect("failed to repair migration checksum");
    }
}

/// Removes duplicate model rows from databases that have not yet run migration 6.
///
/// For databases with one through five applied migrations, this keeps the lowest
/// `model_id` for each `model_sha256`. Other migration states are left unchanged.
///
/// # Errors
///
/// Returns an error if the duplicate rows cannot be deleted.
async fn dedupe_models_before_blob_migration(
    db: &DbContext,
    migration_count: usize,
) -> Result<(), sqlx::Error> {
    if !(1..=5).contains(&migration_count) {
        return Ok(());
    }

    sqlx::query(
        "DELETE FROM models WHERE model_id NOT IN (SELECT MIN(model_id) FROM models GROUP BY model_sha256)",
    )
    .execute(db)
    .await?;

    Ok(())
}

async fn get_db_migration_count(db: &DbContext) -> usize {
    let row: (i64,) = match sqlx::query_as("SELECT COUNT(*) as count FROM _sqlx_migrations")
        .fetch_one(db)
        .await
    {
        Ok(row) => row,
        Err(_) => return 0,
    };

    row.0.try_into().unwrap_or(0)
}

async fn backup_db(db: &DbContext, sqlite_path: &Path, sqlite_backup_dir: &Path) {
    let timestamp = chrono::Utc::now().timestamp_millis();

    if !sqlite_path.exists() {
        return;
    }

    if !sqlite_backup_dir.exists() {
        fs::create_dir_all(sqlite_backup_dir).expect("Failed to create backup directory");
    }

    let backup_file_path = sqlite_backup_dir.join(format!("{timestamp}.sqlite"));
    let escaped_backup_path = backup_file_path.to_string_lossy().replace('\'', "''");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "VACUUM INTO '{escaped_backup_path}'"
    )))
    .execute(db)
    .await
    .expect("Failed to create backup");

    let mut backups: Vec<_> = fs::read_dir(sqlite_backup_dir)
        .expect("Failed to read backup directory")
        .filter_map(|entry| {
            entry.ok().filter(|dir_entry| {
                dir_entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "sqlite")
            })
        })
        .collect();

    backups.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap()
    });
    while backups.len() > 5 {
        let oldest = backups.remove(0);
        fs::remove_file(oldest.path()).expect("Failed to remove old backup");
    }
}

#[cfg(test)]
mod tests {
    /// A pre-migration-6 `models` table with duplicated hashes keeps only the oldest row
    /// per hash; a database past migration 6 (no `model_sha256` column) is left alone.
    #[tokio::test]
    async fn dedupe_keeps_oldest_row_per_hash() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE models (model_id INTEGER PRIMARY KEY, model_sha256 TEXT)")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO models VALUES (1,'a'),(2,'a'),(3,'b'),(4,'a'),(5,'c')")
            .execute(&db)
            .await
            .unwrap();

        super::dedupe_models_before_blob_migration(&db, 6)
            .await
            .unwrap();
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM models")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(count, 5, "must not touch databases past migration 6");

        super::dedupe_models_before_blob_migration(&db, 5)
            .await
            .unwrap();
        let ids: Vec<(i64,)> = sqlx::query_as("SELECT model_id FROM models ORDER BY model_id")
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(ids, vec![(1,), (3,), (5,)]);
    }

    /// A database whose causal writer singleton row is gone must still open.
    #[tokio::test]
    async fn setup_db_opens_without_causal_writer_row() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let backup_dir = dir.path().join("backups");

        let db = super::setup_db(&db_path, &backup_dir).await;
        sqlx::query("DELETE FROM group_causal_replica")
            .execute(&db)
            .await
            .unwrap();
        db.close().await;

        let db = super::setup_db(&db_path, &backup_dir).await;
        db.close().await;
    }
}
