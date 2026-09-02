//! Integration tests for `thumbnail_service` backfill behaviour.
//!
//! The startup backfill re-runs `generate_all_thumbnails` on every launch, so
//! skipping blobs that already have a thumbnail is what keeps it cheap.

use std::{
    fs,
    sync::{Arc, Mutex},
};

use tempfile::tempdir;

use db::{blob_db, db_context, model::user::User};
use service::{
    AppState, Configuration, import_state::ImportState, thumbnail_service::generate_all_thumbnails,
};

/// Builds an `AppState` whose data and app-data paths both point at a fresh
/// temporary directory, so models, thumbnails and the database are isolated
/// per test.
async fn test_app_state() -> (tempfile::TempDir, AppState) {
    let dir = tempdir().unwrap();
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

/// A single triangle is enough for the renderer to produce a real thumbnail.
const TRIANGLE_STL: &str = "solid t
facet normal 0 0 1
outer loop
vertex 0 0 0
vertex 10 0 0
vertex 0 10 0
endloop
endfacet
endsolid t
";

#[tokio::test]
async fn generate_all_thumbnails_leaves_existing_thumbnails_untouched() {
    let (_dir, app_state) = test_app_state().await;

    blob_db::add_blob(&app_state.db, "aa", "stl", 10, None)
        .await
        .unwrap();
    fs::write(app_state.get_model_dir().join("aa.stl"), TRIANGLE_STL).unwrap();

    // Stands in for a thumbnail rendered on an earlier launch.
    let thumbnail = app_state.get_image_dir().join("aa.png");
    fs::write(&thumbnail, b"already rendered").unwrap();

    let mut import_state = ImportState::new(None, false, false, false, User::default());
    generate_all_thumbnails(&app_state, false, &mut import_state)
        .await
        .expect("backfill should succeed");

    assert_eq!(
        fs::read(&thumbnail).unwrap(),
        b"already rendered",
        "an existing thumbnail must not be regenerated on startup"
    );

    // Guards the assertion above: with overwriting on, this model does render,
    // so the untouched bytes are the skip and not a silent failure.
    generate_all_thumbnails(&app_state, true, &mut import_state)
        .await
        .expect("regeneration should succeed");

    assert_ne!(
        fs::read(&thumbnail).unwrap(),
        b"already rendered",
        "overwriting should have replaced the placeholder bytes"
    );
}

#[tokio::test]
async fn generate_all_thumbnails_survives_blobs_without_model_files() {
    let (_dir, app_state) = test_app_state().await;

    blob_db::add_blob(&app_state.db, "missing", "stl", 10, None)
        .await
        .unwrap();

    let mut import_state = ImportState::new(None, false, false, false, User::default());

    // A shared data path can hold rows whose files are not reachable from this
    // machine; the startup backfill must not take the app down over it.
    generate_all_thumbnails(&app_state, false, &mut import_state)
        .await
        .expect("backfill should tolerate unreachable models");

    assert!(!app_state.get_image_dir().join("missing.png").exists());
}
