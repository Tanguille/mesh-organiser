//! Fixtures shared by the crate's unit tests.

/// The body limit axum applies to a route that sets none.
pub const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

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

/// An app state over `data_path` whose pool is lazy and never connects, for requests that never
/// reach the database.
pub fn lazy_app_state(data_path: &std::path::Path) -> service::AppState {
    let data_path = data_path.to_string_lossy().to_string();

    service::AppState {
        db: std::sync::Arc::new(sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap()),
        configuration: std::sync::Mutex::new(service::Configuration {
            data_path: data_path.clone(),
            ..Default::default()
        }),
        import_mutex: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: data_path,
    }
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

/// A local-account (user 1) model whose model file and thumbnail exist on disk, as an import
/// leaves them.
pub async fn add_model_with_file(app_state: &service::AppState, name: &str, sha256: &str) -> i64 {
    let (blob_id, _, _) = add_blob_with_files(app_state, sha256).await;

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

/// `router` behind the session and auth layers `app.rs` installs.
pub async fn with_session(
    router: axum::Router<service::AppState>,
    app_state: service::AppState,
) -> axum::Router {
    let store = crate::session_store::SqliteStore::new(app_state.db.as_ref().clone());
    store.migrate().await.unwrap();
    let auth_layer = axum_login::AuthManagerLayerBuilder::new(
        crate::user::Backend::new(app_state.db.clone()),
        tower_sessions::SessionManagerLayer::new(store),
    )
    .build();

    router.with_state(app_state).layer(auth_layer)
}

/// Adds alice@example.com with a random password and returns it, for `login_request` to log in
/// with. Random rather than a literal, so no test credential is hard-coded.
pub async fn add_login_user(app_state: &service::AppState) -> String {
    let password = db::random_hex_32();
    db::user_db::add_user(&app_state.db, "alice", "alice@example.com", &password)
        .await
        .unwrap();

    password
}

/// A password login for alice@example.com.
pub fn login_request(password: &str) -> axum::http::Request<axum::body::Body> {
    let body = serde_json::json!({ "email": "alice@example.com", "password": password });

    axum::http::Request::post("/api/v1/login/password")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}
