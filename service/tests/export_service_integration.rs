//! Characterization tests for `export_to_temp_folder` and `export_zip_to_temp_folder`.
//!
//! Both write a `metadata.json` when `export_metadata` is on; it must stay the
//! pretty-printed model list, byte-identical between the folder and zip exports.

mod common;

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use db::model::{Model, ModelFlags, label::LabelMeta, model_group::ModelGroupMeta};
use service::{
    AppState,
    export_service::{export_to_temp_folder, export_zip_to_temp_folder},
};

const GEAR_BYTES: &[u8] = b"solid gear\nendsolid gear\n";
const BRACKET_BYTES: &[u8] = b"solid bracket\nendsolid bracket\n";
const GEAR_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BRACKET_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// Writes the blob files and returns a plain-`stl` model and a zipped-`stl.zip`
/// model in the same group, with every optional field populated on one of them.
fn exported_models(app_state: &AppState) -> Vec<Model> {
    let model_dir = app_state.get_model_dir();
    fs::write(model_dir.join(format!("{GEAR_SHA}.stl")), GEAR_BYTES).unwrap();
    let mut zip =
        ZipWriter::new(fs::File::create(model_dir.join(format!("{BRACKET_SHA}.stl.zip"))).unwrap());
    zip.start_file("Bracket.stl", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(BRACKET_BYTES).unwrap();
    zip.finish().unwrap();

    let group = ModelGroupMeta {
        id: 7,
        name: String::from("Robot Arm"),
        created: String::from("2024-01-01 00:00:00"),
        last_modified: String::from("2024-01-03 00:00:00"),
        resource_id: Some(3),
        unique_global_id: String::from("group-7"),
    };

    let mut gear = common::model_with_blob(1, "Gear", GEAR_SHA, "stl");
    gear.blob.size = 25;
    gear.link = Some(String::from("https://example.com/gear"));
    gear.description = Some(String::from("Spur gear, 20 teeth"));
    gear.group = Some(group.clone());
    gear.labels = vec![LabelMeta {
        id: 4,
        name: String::from("Printed parts"),
        color: 0x00ff_00ff,
        unique_global_id: String::from("label-4"),
        last_modified: String::from("2024-01-04 00:00:00"),
    }];
    gear.flags = ModelFlags::Printed | ModelFlags::Favorite;

    let mut bracket = common::model_with_blob(2, "Bracket", BRACKET_SHA, "stl.zip");
    bracket.group = Some(group);

    vec![gear, bracket]
}

fn zip_entries(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut archive = ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut entry = archive.by_index(index).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();

            (entry.name().unwrap().to_string(), bytes)
        })
        .collect()
}

fn sorted_file_names(paths: &[PathBuf]) -> Vec<String> {
    let mut names: Vec<String> = paths
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    names.sort();

    names
}

#[tokio::test]
async fn export_metadata_json_is_identical_in_folder_and_zip_exports() {
    let (_dir, app_state) = common::test_app_state().await;
    app_state.configuration.lock().unwrap().export_metadata = true;
    let expected_metadata = serde_json::to_vec_pretty(&exported_models(&app_state)).unwrap();

    let (folder, paths) =
        export_to_temp_folder(exported_models(&app_state), &app_state, false, "pinning")
            .await
            .unwrap();
    let zip_result = export_zip_to_temp_folder(exported_models(&app_state), &app_state)
        .await
        .unwrap();

    let folder_metadata = fs::read(folder.join("metadata.json")).unwrap();
    assert_eq!(
        String::from_utf8(folder_metadata).unwrap(),
        String::from_utf8(expected_metadata.clone()).unwrap()
    );
    assert_eq!(
        sorted_file_names(&paths),
        vec![String::from("Bracket.stl"), String::from("Gear.stl")]
    );
    assert_eq!(fs::read(folder.join("Gear.stl")).unwrap(), GEAR_BYTES);
    assert_eq!(fs::read(folder.join("Bracket.stl")).unwrap(), BRACKET_BYTES);

    assert_eq!(
        zip_result.zip_path.file_name().unwrap().to_str(),
        Some("Robot Arm.zip")
    );
    assert_eq!(
        zip_entries(&zip_result.zip_path),
        vec![
            (String::from("metadata.json"), expected_metadata),
            (String::from("Gear.stl"), GEAR_BYTES.to_vec()),
            (String::from("Bracket.stl"), BRACKET_BYTES.to_vec()),
        ]
    );

    fs::remove_dir_all(folder).unwrap();
    fs::remove_dir_all(zip_result.temp_dir).unwrap();
}

#[tokio::test]
async fn export_without_metadata_writes_only_models() {
    let (_dir, app_state) = common::test_app_state().await;
    assert!(!app_state.get_configuration().export_metadata);

    let (folder, paths) =
        export_to_temp_folder(exported_models(&app_state), &app_state, true, "pinning")
            .await
            .unwrap();
    let zip_result = export_zip_to_temp_folder(exported_models(&app_state), &app_state)
        .await
        .unwrap();

    assert!(!folder.join("metadata.json").exists());
    // Lazy export hands back plain blobs in place and only unpacks zipped ones.
    let mut paths = paths;
    paths.sort();
    let mut expected_paths = vec![
        app_state.get_model_dir().join(format!("{GEAR_SHA}.stl")),
        folder.join("Bracket.stl"),
    ];
    expected_paths.sort();
    assert_eq!(paths, expected_paths);

    let entry_names: Vec<String> = zip_entries(&zip_result.zip_path)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        entry_names,
        vec![String::from("Gear.stl"), String::from("Bracket.stl")]
    );

    fs::remove_dir_all(folder).unwrap();
    fs::remove_dir_all(zip_result.temp_dir).unwrap();
}
