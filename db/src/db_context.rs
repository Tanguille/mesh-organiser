use std::{fs, path::PathBuf, time::Duration};

use sqlx::{
    self, Pool, Sqlite,
    migrate::MigrateDatabase,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

pub type DbContext = Pool<Sqlite>;

pub async fn setup_db(sqlite_path: &PathBuf, sqlite_backup_dir: &PathBuf) -> DbContext {
    let url = format!(
        "sqlite:{path}",
        path = sqlite_path.to_str().expect("path should be something")
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

    sqlx::migrate!("./migrations").run(&db).await.unwrap();
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

fn backup_db(sqlite_path: &PathBuf, sqlite_backup_dir: &PathBuf) {
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
