//! Integration tests for [`service::slice_service`] (no real OrcaSlicer in CI).

use std::sync::{Arc, Mutex, OnceLock};

use db::{blob_db, model::user::User, model_db};
use service::{
    AppState, Configuration,
    slice_service::{ORCA_SLICER_EXECUTABLE_ENV, SliceOrchestrationSettings, slice_model_for_user},
};
use tempfile::tempdir;

fn slice_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("slice env test lock poisoned")
}

async fn app_state_with_stl_model() -> (tempfile::TempDir, AppState, i64) {
    let dir = tempdir().unwrap();
    let data_path = dir.path().join("data");
    std::fs::create_dir_all(&data_path).unwrap();
    let db_path = data_path.join("db.sqlite");
    let backup_dir = data_path.join("backup");
    std::fs::create_dir_all(&backup_dir).unwrap();

    let db = db::db_context::setup_db(&db_path, &backup_dir).await;
    let config = Configuration {
        data_path: data_path.to_string_lossy().to_string(),
        ..Default::default()
    };

    let app_state = AppState {
        db: Arc::new(db),
        configuration: Mutex::new(config),
        import_mutex: Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: data_path.to_string_lossy().to_string(),
    };

    let model_dir = app_state.get_model_dir();
    let sha256 = "0123456789abcdef0123456789abcdef";
    let filetype = "stl";
    let content = b"solid mock\nendsolid mock\n";
    let path = model_dir.join(format!("{sha256}.{filetype}"));
    std::fs::write(&path, content).unwrap();

    let blob_id = blob_db::add_blob(
        &app_state.db,
        sha256,
        filetype,
        i64::try_from(content.len()).unwrap(),
        None,
    )
    .await
    .unwrap();

    let user = User::default();
    let model_id = model_db::add_model(&app_state.db, &user, "MockPart", blob_id, None, None)
        .await
        .unwrap();

    (dir, app_state, model_id)
}

#[tokio::test]
async fn slice_errors_when_slicer_env_unset() {
    let _guard = slice_env_lock();
    let prior = std::env::var(ORCA_SLICER_EXECUTABLE_ENV).ok();

    // SAFETY: This is test code running in single-threaded context
    unsafe { std::env::remove_var(ORCA_SLICER_EXECUTABLE_ENV) };

    let (_dir, app_state, model_id) = app_state_with_stl_model().await;
    let user = User::default();
    let settings = SliceOrchestrationSettings::default();

    let err = slice_model_for_user(&app_state, &user, model_id, &settings)
        .await
        .expect_err("missing slicer env should error");

    let msg = err.to_string();
    assert!(
        msg.contains("Slicer not configured") || msg.contains(ORCA_SLICER_EXECUTABLE_ENV),
        "unexpected message: {msg}"
    );

    match prior {
        // SAFETY: This is test code running in single-threaded context
        Some(value) => unsafe { std::env::set_var(ORCA_SLICER_EXECUTABLE_ENV, value) },
        None => unsafe { std::env::remove_var(ORCA_SLICER_EXECUTABLE_ENV) },
    }
}

#[tokio::test]
async fn slice_errors_when_slicer_path_missing() {
    let _guard = slice_env_lock();
    let prior = std::env::var(ORCA_SLICER_EXECUTABLE_ENV).ok();

    let ghost = tempdir()
        .unwrap()
        .path()
        .join("nonexistent_orca_binary.exe");
    // SAFETY: This is test code running in single-threaded context
    unsafe { std::env::set_var(ORCA_SLICER_EXECUTABLE_ENV, ghost.as_os_str()) };

    let (_dir, app_state, model_id) = app_state_with_stl_model().await;
    let user = User::default();
    let settings = SliceOrchestrationSettings::default();

    let err = slice_model_for_user(&app_state, &user, model_id, &settings)
        .await
        .expect_err("missing executable should error");

    let msg = err.to_string();
    assert!(
        msg.contains("not found") || msg.contains("executable"),
        "unexpected message: {msg}"
    );

    match prior {
        // SAFETY: This is test code running in single-threaded context
        Some(value) => unsafe { std::env::set_var(ORCA_SLICER_EXECUTABLE_ENV, value) },
        None => unsafe { std::env::remove_var(ORCA_SLICER_EXECUTABLE_ENV) },
    }
}
