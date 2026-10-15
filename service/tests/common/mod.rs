//! Helpers shared by the service integration tests.

// Each integration test crate compiles this module on its own and uses a subset of it.
#![allow(dead_code)]

use std::{
    fs,
    sync::{Arc, Mutex},
};

use tempfile::TempDir;

use db::{
    db_context,
    model::{Model, ModelFlags, blob::Blob},
};
use service::{AppState, Configuration};

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
        ..Default::default()
    };

    let app_state = AppState {
        db: Arc::new(db),
        configuration: Mutex::new(config),
        import_mutex: Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: data_path.to_string_lossy().to_string(),
    };

    (dir, app_state)
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
    }
}
