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

#[cfg(target_os = "linux")]
use std::os::unix::{ffi::OsStrExt, fs::PermissionsExt};

use tempfile::TempDir;
use zip::{ZipWriter, write::SimpleFileOptions};

use db::{label_db, label_keyword_db, model::user::User, model_db};
use service::{
    AppState, ServiceError, export_service,
    import_service::{add_labels_by_keywords, get_model_count, import_path},
    import_state::{ImportState, ImportedModelsSet},
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
        common::internal_error_message(
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
        common::internal_error_message(
            get_model_count(zip_path.to_str().unwrap(), false, true).await
        ),
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

/// Thumbnails and downloads read a stored blob as soon as import returns, so both the zipped and
/// the raw write must be on disk by then.
#[tokio::test]
async fn import_path_stores_a_blob_that_reads_back_intact_right_away() {
    let (_dir, app_state) = common::test_app_state().await;
    let root = TempDir::new().unwrap();

    for (name, filetype, zipped) in [("Zipped.stl", "stl.zip", true), ("Raw.3mf", "3mf", false)] {
        let path = root.path().join(name);
        write_file(&path);
        let result = import_path(
            path.to_str().unwrap(),
            &app_state,
            import_state(false, false, false),
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
        assert_eq!(models[0].blob.filetype, filetype);
        let stored = export_service::get_model_path_for_blob(&models[0].blob, &app_state);

        // Read without yielding, so a write still pending in tokio's blocking pool shows up.
        let bytes = if zipped {
            let mut archive = zip::ZipArchive::new(fs::File::open(&stored).unwrap()).unwrap();
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut archive.by_index(0).unwrap(), &mut bytes).unwrap();

            bytes
        } else {
            fs::read(&stored).unwrap()
        };

        assert_eq!(bytes, name.as_bytes(), "{name}");
    }
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
async fn import_path_zip_reimport_keeps_live_and_trashed_models_in_their_group_and_adds_none() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let root = TempDir::new().unwrap();
    let zip_path = root.path().join("Parts_Pack.ZIP");
    write_zip(&zip_path, ZIP_ENTRIES);
    let import = || {
        import_path(
            zip_path.to_str().unwrap(),
            &app_state,
            import_state(false, false, false),
        )
    };
    let first = import().await.unwrap();
    let group_id = first.imported_models[0].group_id.unwrap();
    let mut model_ids = first.imported_models[0].model_ids.clone();
    model_ids.sort_unstable();
    // The re-import restores the trashed model through the sha256 dedupe.
    export_service::delete_models(&app_state, &user, vec![model_ids[0]])
        .await
        .unwrap();

    let reimport = import().await.unwrap();

    assert_eq!(reimport.imported_models.len(), 1);
    assert_eq!(reimport.imported_models[0].group_id, None);
    let mut reimported_ids = reimport.imported_models[0].model_ids.clone();
    reimported_ids.sort_unstable();
    assert_eq!(reimported_ids, model_ids);
    let models = model_db::get_models_via_ids(&app_state.db, &user, model_ids)
        .await
        .unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model.group.as_ref().map(|group| group.id))
            .collect::<Vec<_>>(),
        vec![Some(group_id); 3]
    );
    let group_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models_group")
        .fetch_one(&*app_state.db)
        .await
        .unwrap();
    assert_eq!(group_count, 1);
}

/// A `My_Folder/<non-UTF-8 name>/part.stl` tree, which Linux allows and macOS refuses.
#[cfg(target_os = "linux")]
fn non_utf8_folder_tree(root: &Path) -> PathBuf {
    let folder = root.join("My_Folder");
    write_file(
        &folder
            .join(std::ffi::OsStr::from_bytes(b"Caf\xe9_Parts"))
            .join("part.stl"),
    );

    folder
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn import_path_recursive_directory_imports_a_folder_with_a_non_utf8_name() {
    let (_dir, app_state) = common::test_app_state().await;
    let root = TempDir::new().unwrap();
    let folder = non_utf8_folder_tree(root.path());
    let folder = folder.to_str().unwrap();

    assert_eq!(get_model_count(folder, true, false).await.unwrap(), 1);
    let result = import_path(folder, &app_state, import_state(true, false, false))
        .await
        .unwrap();

    assert_eq!(
        summarize(&app_state, &result).await,
        vec![(
            true,
            Some(String::from("Caf\u{FFFD} Parts")),
            named(&[("part", "stl.zip")]),
        )]
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn import_path_as_path_skips_a_file_whose_path_is_not_utf8() {
    let (_dir, app_state) = common::test_app_state().await;
    let root = TempDir::new().unwrap();
    let folder = non_utf8_folder_tree(root.path());

    // The blob row stores the referenced path as text, so this file can't be referenced.
    let result = import_path(
        folder.to_str().unwrap(),
        &app_state,
        import_state(true, false, true),
    )
    .await
    .unwrap();

    assert_eq!((result.model_count, result.imported_model_count), (1, 0));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn import_path_recursive_directory_skips_an_unreadable_subfolder_but_not_an_unreadable_root()
{
    let (_dir, app_state) = common::test_app_state().await;
    let root = TempDir::new().unwrap();
    let folder = root.path().join("My_Folder");
    let locked = folder.join("Locked");
    write_file(&folder.join("top.stl"));
    write_file(&locked.join("inner.stl"));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    // A root process ignores the mode bits, so the denial this test needs does not happen.
    if fs::read_dir(&locked).is_ok() {
        return;
    }
    let folder = folder.to_str().unwrap();

    let count = get_model_count(folder, true, false).await;
    let result = import_path(folder, &app_state, import_state(true, false, false)).await;
    let locked_count = get_model_count(locked.to_str().unwrap(), true, false).await;
    // Readable again before any assertion, so the temp dir can still be removed if one fails.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(count.unwrap(), 1);
    assert_eq!(
        summarize(&app_state, &result.unwrap()).await,
        vec![(
            true,
            Some(String::from("My Folder")),
            named(&[("top", "stl.zip")]),
        )]
    );
    assert!(matches!(
        locked_count,
        Err(ServiceError::FileSystemFault(_))
    ));
}

/// Adds a label named after `keyword` that keyword matching applies on import.
async fn add_keyword_label(app_state: &AppState, keyword: &str) -> i64 {
    let user = User::default();
    let label_id = label_db::add_label(&app_state.db, &user, keyword, 0, None)
        .await
        .unwrap()
        .id;
    label_keyword_db::set_keywords_for_label(
        &app_state.db,
        &user,
        label_id,
        vec![keyword.to_string()],
        None,
    )
    .await
    .unwrap();

    label_id
}

/// The label ids of each model the import added.
async fn imported_label_ids(app_state: &AppState, import_state: &ImportState) -> Vec<Vec<i64>> {
    model_db::get_models_via_ids(
        &app_state.db,
        &User::default(),
        import_state.all_model_ids(),
    )
    .await
    .unwrap()
    .into_iter()
    .map(|model| model.labels.iter().map(|label| label.id).collect())
    .collect()
}

#[tokio::test]
async fn import_path_zip_labels_new_models_by_the_group_it_creates() {
    let (_dir, app_state) = common::test_app_state().await;
    let label_id = add_keyword_label(&app_state, "gear").await;
    let root = TempDir::new().unwrap();
    let zip_path = root.path().join("Gear_Pack.zip");
    write_zip(&zip_path, &["wheel.stl", "axle.stl"]);

    let result = import_path(
        zip_path.to_str().unwrap(),
        &app_state,
        import_state(false, false, false),
    )
    .await
    .unwrap();

    assert_eq!(
        imported_label_ids(&app_state, &result).await,
        vec![vec![label_id]; 2]
    );
}

#[tokio::test]
async fn add_labels_by_keywords_labels_every_model_past_the_pair_bind_limit() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let label_id = add_keyword_label(&app_state, "bulk").await;
    // The insert binds two variables per (label, model) pair, so 17,000 models overflow SQLite's
    // 32,766-variable limit in one statement.
    let model_ids = common::add_bulk_models(&app_state, &user, 17_000, None).await;
    let new_models = [ImportedModelsSet {
        group_id: None,
        group_name: None,
        model_ids: model_ids.clone(),
    }];

    add_labels_by_keywords(&new_models, &app_state, &import_state(false, false, false)).await;

    let models = model_db::get_models_via_ids(&app_state.db, &user, model_ids)
        .await
        .unwrap();
    assert_eq!(models.len(), 17_000);
    assert!(
        models
            .iter()
            .all(|model| model.labels.iter().any(|label| label.id == label_id))
    );
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
        common::internal_error_message(
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
        common::internal_error_message(
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
