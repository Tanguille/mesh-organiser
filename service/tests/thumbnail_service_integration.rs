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
