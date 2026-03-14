//! Integration tests for `resource_service` (`open_resource_folder`, `delete_resource_folder`).
//!
//! Locks in sync behaviour after refactor: folders are created/removed as before.

use std::sync::{Arc, Mutex};

use db::db_context;
use db::model::resource::{ResourceFlags, ResourceMeta};
use db::model::user::User;
use service::resource_service::{delete_resource_folder, open_resource_folder};
use service::{AppState, Configuration};
use tempfile::tempdir;

#[tokio::test]
async fn open_resource_folder_creates_folder_when_missing() {
    let dir = tempdir().unwrap();
    let data_path = dir.path().join("data");
    std::fs::create_dir_all(&data_path).unwrap();
    let db_path = data_path.join("db.sqlite");
    let backup_dir = data_path.join("backup");
    std::fs::create_dir_all(&backup_dir).unwrap();

    let db = db_context::setup_db(&db_path, &backup_dir).await;
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

    let resource = ResourceMeta {
        id: 42,
        name: "Test".to_string(),
        flags: ResourceFlags::empty(),
        created: String::new(),
        last_modified: String::new(),
        unique_global_id: String::new(),
    };
    let user = User::default(); // id: 1

    open_resource_folder(&resource, &user, &app_state)
        .expect("open_resource_folder should succeed");

    let resources_dir = app_state.get_resources_dir();
    let resource_folder = resources_dir.join("42_1");
    assert!(
        resource_folder.exists(),
        "resource folder 42_1 should exist after open_resource_folder"
    );
}

#[tokio::test]
async fn delete_resource_folder_removes_folder() {
    let dir = tempdir().unwrap();
    let data_path = dir.path().join("data");
    std::fs::create_dir_all(&data_path).unwrap();
    let db_path = data_path.join("db.sqlite");
    let backup_dir = data_path.join("backup");
    std::fs::create_dir_all(&backup_dir).unwrap();

    let db = db_context::setup_db(&db_path, &backup_dir).await;
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

    let resources_dir = app_state.get_resources_dir();
    let resource_folder = resources_dir.join("7_1");
    std::fs::create_dir_all(&resource_folder).unwrap();
    assert!(resource_folder.exists());

    let resource = ResourceMeta {
        id: 7,
        name: "Del".to_string(),
        flags: ResourceFlags::empty(),
        created: String::new(),
        last_modified: String::new(),
        unique_global_id: String::new(),
    };
    let user = User::default();

    delete_resource_folder(&resource, &user, &app_state)
        .expect("delete_resource_folder should succeed");

    assert!(
        !resource_folder.exists(),
        "resource folder 7_1 should be removed after delete_resource_folder"
    );
}
