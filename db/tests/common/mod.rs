use db::db_context;

pub async fn test_db() -> (tempfile::TempDir, db_context::DbContext) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("db.sqlite");
    let backup_dir = dir.path().join("backups");
    std::fs::create_dir_all(&backup_dir).unwrap();
    let db = db_context::setup_db(&db_path, &backup_dir).await;

    (dir, db)
}
