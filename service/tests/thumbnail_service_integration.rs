//! Integration tests for `thumbnail_service` backfill behaviour.
//!
//! The startup backfill re-runs `generate_all_thumbnails` on every launch, so
//! skipping blobs that already have a thumbnail is what keeps it cheap.

mod common;

use std::fs;

use db::{blob_db, model::user::User};
use service::{import_state::ImportState, thumbnail_service::generate_all_thumbnails};

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
    let (_dir, app_state) = common::test_app_state().await;

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
    let (_dir, app_state) = common::test_app_state().await;

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

/// Imports the cube STEP fixture (stored zipped, as import stores it), generates
/// thumbnails with the given STEP preview cap in MB, and reports whether one was written.
#[cfg(feature = "step")]
async fn step_fixture_gets_thumbnail(max_size_model_step_preview: u32) -> bool {
    let (_dir, app_state) = common::test_app_state().await;
    app_state
        .configuration
        .lock()
        .unwrap()
        .max_size_model_step_preview = max_size_model_step_preview;
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../libmeshthumbnail/tests/fixtures/cube.step");
    let imported = service::import_service::import_path(
        fixture.to_str().unwrap(),
        &app_state,
        ImportState::new(None, false, false, false, User::default()),
    )
    .await
    .unwrap();

    let mut import_state = ImportState::new(None, false, false, false, User::default());
    let models = service::thumbnail_service::generate_thumbnails_for_model_ids(
        &app_state,
        &User::default(),
        imported.imported_models[0].model_ids.clone(),
        &mut import_state,
    )
    .await
    .expect("thumbnail generation should succeed");
    assert_eq!(models[0].blob.filetype, "step.zip");
    // The stored blob parses, so a missing thumbnail is the gate or the cap, not the fixture.
    let model_path = service::export_service::get_model_path_for_blob(&models[0].blob, &app_state);
    assert!(
        libmeshthumbnail::parse_model::handle_parse(&model_path)
            .unwrap()
            .is_some()
    );

    app_state
        .get_image_dir()
        .join(format!("{}.png", models[0].blob.sha256))
        .exists()
}

#[cfg(feature = "step")]
#[tokio::test]
async fn step_model_under_size_cap_gets_thumbnail() {
    assert!(step_fixture_gets_thumbnail(10).await);
}

#[cfg(feature = "step")]
#[tokio::test]
async fn step_model_over_size_cap_gets_no_thumbnail() {
    // A 0 MB cap puts every non-empty STEP blob over it.
    assert!(!step_fixture_gets_thumbnail(0).await);
}

/// Both thumbnail paths, the rendered mesh and the resized embedded 3MF image,
/// produce images of the service's fixed 400x400 thumbnail size.
#[tokio::test]
async fn generated_thumbnails_are_400_by_400() {
    let (_dir, app_state) = common::test_app_state().await;

    blob_db::add_blob(&app_state.db, "rendered", "stl", 10, None)
        .await
        .unwrap();
    fs::write(app_state.get_model_dir().join("rendered.stl"), TRIANGLE_STL).unwrap();

    // Defaults prefer embedded 3MF images; a wide input proves resizing fills the square.
    let mut embedded_png = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(40, 20))
        .write_to(
            &mut std::io::Cursor::new(&mut embedded_png),
            image::ImageFormat::Png,
        )
        .unwrap();
    blob_db::add_blob(&app_state.db, "embedded", "3mf", 10, None)
        .await
        .unwrap();
    let mut threemf = zip::ZipWriter::new(
        fs::File::create(app_state.get_model_dir().join("embedded.3mf")).unwrap(),
    );
    threemf
        .start_file(
            "Metadata/thumbnail_middle.png",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    std::io::Write::write_all(&mut threemf, &embedded_png).unwrap();
    threemf.finish().unwrap();

    let mut import_state = ImportState::new(None, false, false, false, User::default());
    generate_all_thumbnails(&app_state, false, &mut import_state)
        .await
        .expect("thumbnail generation should succeed");

    for name in ["rendered", "embedded"] {
        let thumbnail = app_state.get_image_dir().join(format!("{name}.png"));
        assert_eq!(
            image::image_dimensions(&thumbnail).unwrap(),
            (400, 400),
            "{name} thumbnail size"
        );
    }
}
