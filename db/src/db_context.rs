use std::{fs, path::Path, time::Duration};

use sha2::{Digest, Sha384};
use sqlx::{
    self, Pool, Sqlite,
    migrate::{MigrateDatabase, Migrator},
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

pub type DbContext = Pool<Sqlite>;

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

    let connection_option = SqliteConnectOptions::new()
        .filename(sqlite_path)
        .busy_timeout(Duration::from_secs(15));

    let db = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connection_option)
        .await
        .unwrap();

    let migration_count = get_db_migration_count(&db).await;

    let migrator = sqlx::migrate!("./migrations");
    repair_line_ending_checksums(&db, &migrator).await;
    migrator
        .run(&db)
        .await
        .expect("failed to run database migrations");
    backup_db(sqlite_path, sqlite_backup_dir);

    let new_migration_count = get_db_migration_count(&db).await;

    if new_migration_count > migration_count {
        sqlx::query!("VACUUM")
            .execute(&db)
            .await
            .expect("Failed to vacuum database after migrations");
    }

    db
}

/// Rewrites stored migration checksums that differ from the embedded migrations
/// only in line endings.
///
/// sqlx hashes the raw bytes of each migration file, so a CRLF checkout and an
/// LF one disagree about byte-identical SQL — and the database can be shared
/// between platforms through a network data path. Checksums matching neither
/// variant are left alone, so a genuinely modified migration still fails.
async fn repair_line_ending_checksums(db: &DbContext, migrator: &Migrator) {
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
        let Some((_, stored)) = applied.iter().find(|(v, _)| *v == migration.version) else {
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

async fn get_db_migration_count(db: &DbContext) -> usize {
    let row: (i64,) = match sqlx::query_as("SELECT COUNT(*) as count FROM _sqlx_migrations")
        .fetch_one(db)
        .await
    {
        Ok(r) => r,
        Err(_) => return 0,
    };

    row.0.try_into().unwrap_or(0)
}

fn backup_db(sqlite_path: &Path, sqlite_backup_dir: &Path) {
    let timestamp = chrono::Utc::now().timestamp_millis();

    if !sqlite_path.exists() {
        return;
    }

    if !sqlite_backup_dir.exists() {
        fs::create_dir_all(sqlite_backup_dir).expect("Failed to create backup directory");
    }

    let backup_file_path = sqlite_backup_dir.join(format!("{timestamp}.sqlite"));
    fs::copy(sqlite_path, &backup_file_path).expect("Failed to create backup");

    let mut backups: Vec<_> = fs::read_dir(sqlite_backup_dir)
        .expect("Failed to read backup directory")
        .filter_map(|entry| {
            entry
                .ok()
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "sqlite"))
        })
        .collect();

    backups.sort_by_key(|entry| entry.metadata().and_then(|m| m.modified()).unwrap());
    while backups.len() > 5 {
        let oldest = backups.remove(0);
        fs::remove_file(oldest.path()).expect("Failed to remove old backup");
    }
}
