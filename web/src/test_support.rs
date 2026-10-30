//! Fixtures shared by the crate's unit tests.

/// An app state over a fresh, migrated database and data dir; the `TempDir` must outlive the
/// test. The migrations seed the local account as user 1, which `User::default()` names.
pub async fn app_state() -> (tempfile::TempDir, service::AppState) {
    let dir = tempfile::TempDir::new().unwrap();
    let data_path = dir.path().to_string_lossy().to_string();
    let backup_dir = dir.path().join("backups");
    std::fs::create_dir_all(&backup_dir).unwrap();
    let db = db::db_context::setup_db(&dir.path().join("db.sqlite"), &backup_dir).await;
    let app_state = service::AppState {
        db: std::sync::Arc::new(db),
        configuration: std::sync::Mutex::new(service::Configuration {
            data_path: data_path.clone(),
            ..Default::default()
        }),
        import_mutex: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: data_path,
    };

    (dir, app_state)
}

/// Adds a blob whose model file and thumbnail exist on disk, as an import leaves them.
pub async fn add_blob_with_files(
    app_state: &service::AppState,
    sha256: &str,
) -> (i64, std::path::PathBuf, std::path::PathBuf) {
    let blob_id = db::blob_db::add_blob(&app_state.db, sha256, "stl", 1, None)
        .await
        .unwrap();
    let blob = db::blob_db::get_blob_via_sha256(&app_state.db, sha256)
        .await
        .unwrap()
        .unwrap();
    let model_path = service::export_service::get_model_path_for_blob(&blob, app_state);
    let image_path = service::export_service::get_image_path_for_blob(&blob, app_state);
    std::fs::write(&model_path, sha256).unwrap();
    std::fs::write(&image_path, sha256).unwrap();

    (blob_id, model_path, image_path)
}

/// A local-account (user 1) model whose file exists on disk, as an import leaves it.
pub async fn add_model_with_file(app_state: &service::AppState, name: &str, sha256: &str) -> i64 {
    let blob_id = db::blob_db::add_blob(&app_state.db, sha256, "stl", 1, None)
        .await
        .unwrap();
    let blob = db::blob_db::get_blob_via_sha256(&app_state.db, sha256)
        .await
        .unwrap()
        .unwrap();
    std::fs::write(
        service::export_service::get_model_path_for_blob(&blob, app_state),
        name,
    )
    .unwrap();

    db::model_db::add_model(
        &app_state.db,
        &db::model::user::User::default(),
        name,
        blob_id,
        None,
        None,
    )
    .await
    .unwrap()
}
