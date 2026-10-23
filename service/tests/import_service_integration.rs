//! Characterization tests for the path dispatch shared by `get_model_count` and
//! `import_path`: directory (flat and recursive), zip, single supported file and
//! unsupported file.
//!
//! Model contents are just their file names: import never parses them, and
//! distinct bytes keep the sha256 de-duplication from merging models.

mod common;

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use tempfile::TempDir;
use zip::{ZipWriter, write::SimpleFileOptions};

use db::{model::user::User, model_db};
use service::{
    AppState, ServiceError,
    import_service::{get_model_count, import_path},
    import_state::ImportState,
};

fn write_file(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, path.file_name().unwrap().as_encoded_bytes()).unwrap();
}

/// Writes a zip whose file entries contain their own names; names ending in `/`
/// become directory entries.
fn write_zip(path: &Path, entry_names: &[&str]) {
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    for name in entry_names {
        if name.ends_with('/') {
            zip.add_directory(*name, SimpleFileOptions::default())
                .unwrap();
        } else {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(name.as_bytes()).unwrap();
        }
    }
    zip.finish().unwrap();
}

/// Lays out a `My_Folder` tree mixing supported, unsupported and nested files:
///
/// ```text
/// My_Folder/
///   first.stl, second.OBJ, notes.txt, archive.zip, packed.stl.zip
///   Nested_Dir/third.gcode
///   Nested_Dir/Deeper/fourth.step, fifth.3mf
///   no_models/readme.txt
/// ```
fn model_tree(root: &Path) -> PathBuf {
    let folder = root.join("My_Folder");
    for relative in [
        "first.stl",
        "second.OBJ",
        "notes.txt",
        "Nested_Dir/third.gcode",
        "Nested_Dir/Deeper/fourth.step",
        "Nested_Dir/Deeper/fifth.3mf",
        "no_models/readme.txt",
    ] {
        write_file(&folder.join(relative));
    }
    write_zip(&folder.join("archive.zip"), &["inside.stl"]);
    write_zip(&folder.join("packed.stl.zip"), &["inside.stl"]);

    folder
}

const ZIP_ENTRIES: &[&str] = &[
    "a_part.stl",
    "folder/",
    "folder/b-part.obj",
    "c.txt",
    "d_part.STEP",
    "e.stl.zip",
];

fn internal_error_message<T>(result: Result<T, ServiceError>) -> String {
    match result {
        Err(ServiceError::InternalError(message)) => message,
        Err(e) => panic!("expected InternalError, got {e:?}"),
        Ok(_) => panic!("expected InternalError, got Ok"),
    }
}

#[tokio::test]
async fn get_model_count_dispatches_on_path_kind() {
    let root = TempDir::new().unwrap();
    let folder = model_tree(root.path());
    let zip_path = root.path().join("Parts_Pack.ZIP");
    write_zip(&zip_path, ZIP_ENTRIES);
    let folder = folder.to_str().unwrap();
    let single_file = root.path().join("My_Folder/first.stl");

    assert_eq!(get_model_count(folder, false, false).await.unwrap(), 2);
    assert_eq!(get_model_count(folder, true, false).await.unwrap(), 5);
    assert_eq!(get_model_count(folder, true, true).await.unwrap(), 5);
    assert_eq!(
        get_model_count(zip_path.to_str().unwrap(), false, false)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        get_model_count(single_file.to_str().unwrap(), true, true)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        internal_error_message(
            get_model_count(
                root.path().join("My_Folder/notes.txt").to_str().unwrap(),
                false,
                false
            )
            .await
        ),
        "Unsupported file type"
    );
    assert_eq!(
        internal_error_message(get_model_count(zip_path.to_str().unwrap(), false, true).await),
        "Cannot import a zip as path"
    );
}

/// One entry per import set: whether a group was created, its name, and the
/// `(model name, blob filetype)` of each model, sorted because models within a
/// directory import concurrently. Sets retain the sequential children-first order.
type SetSummary = (bool, Option<String>, Vec<(String, String)>);

async fn summarize(app_state: &AppState, import_state: &ImportState) -> Vec<SetSummary> {
    let mut summary = Vec::new();
    for set in &import_state.imported_models {
        let models =
            model_db::get_models_via_ids(&app_state.db, &User::default(), set.model_ids.clone())
                .await
                .unwrap();
        assert_eq!(models.len(), set.model_ids.len());
        let mut models: Vec<(String, String)> = models
            .into_iter()
            .map(|model| (model.name, model.blob.filetype))
            .collect();
        models.sort();
        summary.push((set.group_id.is_some(), set.group_name.clone(), models));
    }

    summary
}

fn import_state(recursive: bool, delete_after_import: bool, import_as_path: bool) -> ImportState {
    ImportState::new(
        None,
        recursive,
        delete_after_import,
        import_as_path,
        User::default(),
    )
}

fn named(models: &[(&str, &str)]) -> Vec<(String, String)> {
    models
        .iter()
        .map(|(name, filetype)| ((*name).to_string(), (*filetype).to_string()))
        .collect()
}

#[tokio::test]
async fn import_path_single_file_imports_one_ungrouped_model() {
    let (_dir, app_state) = common::test_app_state().await;
    let root = TempDir::new().unwrap();
    let path = root.path().join("Gear_Wheel-v2.stl");
    write_file(&path);

    let result = import_path(
        path.to_str().unwrap(),
        &app_state,
        import_state(false, false, false),
    )
    .await
    .unwrap();

    assert_eq!(
        (
            result.model_count,
            result.imported_model_count,
            result.status.to_string()
        ),
        (1, 1, String::from("FinishedModels"))
    );
    assert_eq!(
        summarize(&app_state, &result).await,
        vec![(false, None, named(&[("Gear Wheel v2", "stl.zip")]))]
    );
    assert!(path.exists(), "source is kept without delete_after_import");
}

#[tokio::test]
async fn import_path_single_file_as_path_references_source() {
    let (_dir, app_state) = common::test_app_state().await;
    let root = TempDir::new().unwrap();
    let path = root.path().join("Referenced.3mf");
    write_file(&path);

    let result = import_path(
        path.to_str().unwrap(),
        &app_state,
        import_state(false, false, true),
    )
    .await
    .unwrap();

    let models = model_db::get_models_via_ids(
        &app_state.db,
        &User::default(),
        result.imported_models[0].model_ids.clone(),
    )
    .await
    .unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].name, "Referenced");
    assert_eq!(models[0].blob.filetype, "3mf");
    assert_eq!(
        models[0].blob.disk_path.as_deref(),
        Some(path.to_str().unwrap())
    );
}

#[tokio::test]
async fn import_path_directory_groups_top_level_models_only() {
    let (_dir, app_state) = common::test_app_state().await;
    let root = TempDir::new().unwrap();
    let folder = model_tree(root.path());

    let result = import_path(
        folder.to_str().unwrap(),
        &app_state,
        import_state(false, false, false),
    )
    .await
    .unwrap();

    assert_eq!((result.model_count, result.imported_model_count), (2, 2));
    assert_eq!(
        summarize(&app_state, &result).await,
        vec![(
            true,
            Some(String::from("My Folder")),
            named(&[("first", "stl.zip"), ("second", "obj.zip")]),
        )]
    );
}

#[tokio::test]
async fn import_path_recursive_directory_groups_each_folder_children_first() {
    let (_dir, app_state) = common::test_app_state().await;
    let root = TempDir::new().unwrap();
    let folder = model_tree(root.path());

    let result = import_path(
        folder.to_str().unwrap(),
        &app_state,
        import_state(true, false, false),
    )
    .await
    .unwrap();

    assert_eq!((result.model_count, result.imported_model_count), (5, 5));
    assert_eq!(
        summarize(&app_state, &result).await,
        vec![
            (
                true,
                Some(String::from("Deeper")),
                named(&[("fifth", "3mf"), ("fourth", "step.zip")]),
            ),
            (
                true,
                Some(String::from("Nested Dir")),
                named(&[("third", "gcode.zip")]),
            ),
            (
                true,
                Some(String::from("My Folder")),
                named(&[("first", "stl.zip"), ("second", "obj.zip")]),
            ),
        ]
    );
}

#[tokio::test]
async fn import_path_zip_groups_supported_entries_under_zip_name() {
    let (_dir, app_state) = common::test_app_state().await;
    let root = TempDir::new().unwrap();
    let zip_path = root.path().join("Parts_Pack.ZIP");
    write_zip(&zip_path, ZIP_ENTRIES);

    let result = import_path(
        zip_path.to_str().unwrap(),
        &app_state,
        import_state(false, true, false),
    )
    .await
    .unwrap();

    assert_eq!((result.model_count, result.imported_model_count), (3, 3));
    assert_eq!(
        summarize(&app_state, &result).await,
        vec![(
            true,
            Some(String::from("Parts Pack")),
            named(&[
                ("a part", "stl.zip"),
                ("b part", "obj.zip"),
                ("d part", "step.zip"),
            ]),
        )]
    );
    assert!(!zip_path.exists(), "delete_after_import removes the zip");
}

#[tokio::test]
async fn import_path_rejects_unsupported_and_conflicting_options() {
    let (_dir, app_state) = common::test_app_state().await;
    let root = TempDir::new().unwrap();
    let unsupported = root.path().join("notes.txt");
    write_file(&unsupported);
    let supported = root.path().join("model.stl");
    write_file(&supported);

    assert_eq!(
        internal_error_message(
            import_path(
                unsupported.to_str().unwrap(),
                &app_state,
                import_state(false, false, false),
            )
            .await
        ),
        "Unsupported file type"
    );
    assert_eq!(
        internal_error_message(
            import_path(
                supported.to_str().unwrap(),
                &app_state,
                import_state(false, true, true),
            )
            .await
        ),
        "Cannot delete after import when importing as path"
    );
    assert!(supported.exists());
}
