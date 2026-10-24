//! Integration tests for `resource_service` (`ensure_resource_folder`, `delete_resource_folder`).
//!
//! Locks in sync behaviour after refactor: folders are created/removed as before.

mod common;

use db::{
    model::resource::{ResourceFlags, ResourceMeta},
    model::user::User,
};
use service::resource_service::{delete_resource_folder, ensure_resource_folder};

#[tokio::test]
async fn ensure_resource_folder_creates_folder_when_missing() {
    let (_dir, app_state) = common::test_app_state().await;

    let resource = ResourceMeta {
        id: 42,
        name: "Test".to_string(),
        flags: ResourceFlags::empty(),
        created: String::new(),
        last_modified: String::new(),
        unique_global_id: String::new(),
    };
    let user = User::default(); // id: 1

    let returned_path = ensure_resource_folder(&resource, &user, &app_state)
        .expect("ensure_resource_folder should succeed");

    let resources_dir = app_state.get_resources_dir();
    let resource_folder = resources_dir.join("42_1");
    assert_eq!(returned_path, resource_folder);
    assert!(
        resource_folder.exists(),
        "resource folder 42_1 should exist after ensure_resource_folder"
    );
}

#[tokio::test]
async fn delete_resource_folder_removes_folder() {
    let (_dir, app_state) = common::test_app_state().await;

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
