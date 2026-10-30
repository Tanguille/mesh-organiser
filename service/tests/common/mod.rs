//! Helpers shared by the service integration tests.

// Each integration test crate compiles this module on its own and uses a subset of it.
#![allow(dead_code)]

use std::{
    fs,
    sync::{Arc, Mutex},
};

use tempfile::TempDir;

use db::{
    blob_db, db_context,
    model::{Model, ModelFlags, blob::Blob, user::User},
};
use service::{AppState, Configuration, ServiceError};

/// Adds `count` models of `user` sharing one blob, trashed at `deleted` when given, and returns
/// their ids in order. One statement: a thousand single inserts would dominate the run time.
pub async fn add_bulk_models(
    app_state: &AppState,
    user: &User,
    count: u32,
    deleted: Option<&str>,
) -> Vec<i64> {
    let blob_id = blob_db::add_blob(&app_state.db, "bulk", "stl", 1, None)
        .await
        .unwrap();
    sqlx::query(
        "WITH RECURSIVE counter(number) AS (SELECT 1 UNION ALL SELECT number + 1 FROM counter WHERE number < ?)
         INSERT INTO models (model_name, model_blob_id, model_added, model_user_id, model_deleted)
         SELECT 'bulk-' || number, ?, '2024-01-01T00:00:00Z', ?, ? FROM counter",
    )
    .bind(count)
    .bind(blob_id)
    .bind(user.id)
    .bind(deleted)
    .execute(&*app_state.db)
    .await
    .unwrap();

    sqlx::query_scalar("SELECT model_id FROM models WHERE model_blob_id = ? ORDER BY model_id")
        .bind(blob_id)
        .fetch_all(&*app_state.db)
        .await
        .unwrap()
}

/// Builds an `AppState` whose data and app-data paths both point at a fresh
/// temporary directory, so models, thumbnails and the database are isolated per test.
pub async fn test_app_state() -> (TempDir, AppState) {
    let dir = TempDir::new().unwrap();
    let data_path = dir.path().join("data");
    let backup_dir = data_path.join("backup");
    fs::create_dir_all(&backup_dir).unwrap();

    let db = db_context::setup_db(&data_path.join("db.sqlite"), &backup_dir).await;
    let config = Configuration {
        data_path: data_path.to_string_lossy().to_string(),
        ..Configuration::default()
    };

    let app_state = AppState {
        db: Arc::new(db),
        configuration: Mutex::new(config),
        import_mutex: Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: data_path.to_string_lossy().to_string(),
    };

    (dir, app_state)
}

pub fn internal_error_message<T>(result: Result<T, ServiceError>) -> String {
    match result {
        Err(ServiceError::InternalError(message)) => message,
        Err(e) => panic!("expected InternalError, got {e:?}"),
        Ok(_) => panic!("expected InternalError, got Ok"),
    }
}

/// A model whose blob lives in the app's model dir as `<sha256>.<filetype>`.
pub fn model_with_blob(id: i64, name: &str, sha256: &str, filetype: &str) -> Model {
    Model {
        id,
        name: name.to_string(),
        blob: Blob {
            id,
            sha256: sha256.to_string(),
            filetype: filetype.to_string(),
            size: 0,
            added: String::from("2024-01-01 00:00:00"),
            disk_path: None,
        },
        link: None,
        description: None,
        added: String::from("2024-01-01 00:00:00"),
        last_modified: String::from("2024-01-02 00:00:00"),
        group: None,
        labels: Vec::new(),
        flags: ModelFlags::empty(),
        unique_global_id: format!("model-{id}"),
        deleted: None,
    }
}
