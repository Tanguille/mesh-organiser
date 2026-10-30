use std::{
    fs::{self, read_dir},
    path::{Path, PathBuf},
    sync::Arc,
};

use async_zip::{
    ZipEntryBuilder,
    tokio::{read::seek::ZipFileReader, write::ZipFileWriter},
};
use indexmap::IndexMap;
use itertools::Itertools;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::{
    fs::File,
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::Mutex,
};
use tokio_util::compat::FuturesAsyncReadCompatExt;

use db::{
    blob_db, label_db, label_keyword_db,
    model::{blob::FileType, user::User},
    model_db,
};

use crate::{
    ASYNC_MULT,
    import_state::{ImportState, ImportStatus, ImportedModelsSet},
    service_error::ServiceError,
    util::{self, convert_extension_to_zip},
};

use super::app_state::AppState;

/// Imports models from a path (file, directory, or zip).
pub async fn import_path(
    path: &str,
    app_state: &AppState,
    mut import_state: ImportState,
) -> Result<ImportState, ServiceError> {
    if import_state.delete_after_import && import_state.import_as_path {
        return Err(ServiceError::InternalError(String::from(
            "Cannot delete after import when importing as path",
        )));
    }

    let _lock = app_state.import_mutex.lock().await;
    import_state.status = ImportStatus::ProcessingModels;
    import_state.emit_all();

    let model_count =
        get_model_count(path, import_state.recursive, import_state.import_as_path).await?;
    import_state.update_total_model_count(model_count);

    let import_state = Arc::new(Mutex::new(import_state));
    let import_state_clone = Arc::clone(&import_state);

    match import_path_inner(path, app_state, import_state).await {
        Ok(()) => {
            let mut import_state = import_state_clone.lock().await;
            import_state.update_status(ImportStatus::FinishedModels);
            import_state.create_groups_from_all_sets(app_state).await?;
            // After grouping, so a model also matches keywords in the name of a group this import
            // created. An online-account folder import uploads its files one by one and groups
            // them on the client afterwards, so those folder names are not matched.
            add_labels_by_keywords(&import_state.imported_models, app_state, &import_state).await;

            let import_state = {
                let fake = ImportState::new(None, false, false, false, User::default());
                std::mem::replace(&mut *import_state, fake)
            };

            Ok(import_state)
        }
        Err(application_error) => {
            import_state_clone
                .lock()
                .await
                .set_failure(application_error.to_string());
            Err(application_error)
        }
    }
}

/// Returns the number of models at the path (file, directory, or zip).
pub async fn get_model_count(
    path: &str,
    recursive: bool,
    import_as_path: bool,
) -> Result<usize, ServiceError> {
    let path_buff = PathBuf::from(path);

    if path_buff.is_dir() {
        if recursive {
            get_model_count_from_dir_recursive(&path_buff)
        } else {
            get_model_count_from_dir(&path_buff)
        }
    } else if is_zip_path(&path_buff) {
        if import_as_path {
            return Err(ServiceError::InternalError(String::from(
                "Cannot import a zip as path",
            )));
        }
        get_model_count_from_zip(path).await
    } else if is_supported_extension(&path_buff) {
        Ok(1)
    } else {
        Err(ServiceError::InternalError(String::from(
            "Unsupported file type",
        )))
    }
}

/// Inner import implementation; updates `import_state` in place.
async fn import_path_inner(
    path: &str,
    app_state: &AppState,
    import_state: Arc<Mutex<ImportState>>,
) -> Result<(), ServiceError> {
    let path_buff = PathBuf::from(path);
    let name = util::prettify_file_name(&path_buff, path_buff.is_dir());

    let recursive = {
        let import_state = import_state.lock().await;
        import_state.recursive
    };

    if path_buff.is_dir() {
        if recursive {
            import_models_from_dir_recursive(&path_buff, app_state, import_state).await?;
        } else {
            import_models_from_dir(&path_buff, app_state, import_state, name).await?;
        }
    } else if is_zip_path(&path_buff) {
        import_models_from_zip(path, app_state, import_state, name).await?;
    } else if is_supported_extension(&path_buff) {
        import_models_from_dir_inner(app_state, path_buff, import_state).await?;
    } else {
        return Err(ServiceError::InternalError(String::from(
            "Unsupported file type",
        )));
    }

    Ok(())
}

pub async fn add_labels_by_keywords(
    new_models: &[ImportedModelsSet],
    app_state: &AppState,
    import_state: &ImportState,
) {
    let db = &app_state.db;
    let model_ids = new_models
        .iter()
        .flat_map(|r| r.model_ids.iter())
        .copied()
        .collect::<Vec<i64>>();

    let Ok(models) = model_db::get_models_via_ids(db, &import_state.user, model_ids).await else {
        return;
    };

    let Ok(all_keywords) = label_keyword_db::get_all_keywords(db, &import_state.user).await else {
        return;
    };

    let mut all_keywords_map: IndexMap<String, Vec<i64>> = IndexMap::new();

    for (label_id, keywords) in all_keywords {
        for keyword in keywords {
            all_keywords_map
                .entry(keyword.name)
                .or_default()
                .push(label_id);
        }
    }

    // Invert model -> labels into label -> models so we can batch-insert once per
    // distinct label instead of issuing one write per imported model.
    let mut label_to_models: IndexMap<i64, Vec<i64>> = IndexMap::new();

    for model in &models {
        let mut name_parts: Vec<String> = model
            .name
            .split(|c: char| !c.is_alphanumeric())
            .map(str::to_lowercase)
            .collect();

        if let Some(group) = &model.group {
            name_parts.extend(
                group
                    .name
                    .split(|c: char| !c.is_alphanumeric())
                    .map(str::to_lowercase),
            );
        }

        let label_ids: Vec<i64> = name_parts
            .iter()
            .flat_map(|part| all_keywords_map.get(part).into_iter().flatten().copied())
            .filter(|l| {
                !model
                    .labels
                    .iter()
                    .any(|existing_labels| existing_labels.id == *l)
            })
            .unique()
            .collect();

        for label_id in label_ids {
            label_to_models.entry(label_id).or_default().push(model.id);
        }
    }

    for (label_id, model_ids) in &label_to_models {
        // One bind per id in `ensure_owned` and two per pair in the insert, so 10,000 ids per
        // statement stays under SQLite's 32,766-variable limit.
        for chunk in model_ids.chunks(10_000) {
            let _ =
                label_db::add_labels_on_models(db, &import_state.user, &[*label_id], chunk, None)
                    .await;
        }
    }
}

async fn import_models_from_dir_recursive(
    path: &Path,
    app_state: &AppState,
    import_state: Arc<Mutex<ImportState>>,
) -> Result<(), ServiceError> {
    // A listing that fails partway is skipped like one that can't be opened.
    let Ok(entries) = read_dir(path).and_then(Iterator::collect::<Result<Vec<_>, _>>) else {
        return Ok(());
    };

    for folder in entries.iter().filter(|f| f.path().is_dir()) {
        let import_state = Arc::clone(&import_state);
        Box::pin(import_models_from_dir_recursive(
            &folder.path(),
            app_state,
            import_state,
        ))
        .await?;
    }

    if entries
        .iter()
        .filter(|f| f.path().is_file())
        .any(|f| is_supported_extension(&f.path()))
    {
        let group_name = util::prettify_file_name(path, true);

        let _ = import_models_from_dir(path, app_state, import_state, group_name).await;
    }

    Ok(())
}

async fn import_models_from_dir_inner(
    app_state: &AppState,
    path: PathBuf,
    import_state_mutex: Arc<Mutex<ImportState>>,
) -> Result<(), ServiceError> {
    if !is_supported_extension(&path) {
        return Err(ServiceError::InternalError("Unsupported filetype".into()));
    }

    // Snapshot the settings so the lock is not held across the import; they are fixed once the
    // import starts, so reading them per file is equivalent to reading them once.
    let (user, link, delete_after_import, import_as_path) = {
        let import_state = import_state_mutex.lock().await;
        (
            import_state.user.clone(),
            import_state.origin_url.clone(),
            import_state.delete_after_import,
            import_state.import_as_path,
        )
    };

    let file_name = util::prettify_file_name(&path, false);
    // Extension unwraps are guarded by the is_supported_extension check above;
    // metadata can still fail at runtime (file removed/unreadable), so propagate.
    let extension = path.extension().unwrap().to_str().unwrap();
    let file_size = usize::try_from(path.metadata()?.len()).unwrap_or(0);

    let mut file = File::open(&path).await?;
    let permanent_disk_path = import_as_path.then(|| path.clone());

    let id = import_single_model(
        &mut file,
        extension,
        file_size,
        &file_name,
        link.as_deref(),
        app_state,
        &user,
        permanent_disk_path,
    )
    .await?;

    {
        let import_state = &mut import_state_mutex.lock().await;
        import_state.add_model_id_to_current_set(id);
    }

    if delete_after_import {
        let _ = fs::remove_file(&path);
    }

    Ok(())
}

async fn import_models_from_dir(
    path: &Path,
    app_state: &AppState,
    import_state: Arc<Mutex<ImportState>>,
    group_name: String,
) -> Result<(), ServiceError> {
    let configuration = app_state.get_configuration();
    import_state
        .lock()
        .await
        .add_new_import_set(Some(group_name));

    let entries: Vec<PathBuf> = read_dir(path)?
        .map_ok(|entry| entry.path())
        .filter_ok(|path| path.is_file())
        .collect::<Result<_, _>>()?;

    let max = configuration.core_parallelism * ASYNC_MULT;

    // Per-entry task results are intentionally ignored.
    let _ = util::run_bounded(entries, max, |entry| {
        let app_state = app_state.clone();
        let import_state_mutex = Arc::clone(&import_state);

        async move { import_models_from_dir_inner(&app_state, entry, import_state_mutex).await }
    })
    .await;

    Ok(())
}

async fn import_models_from_zip(
    path: &str,
    app_state: &AppState,
    import_state: Arc<Mutex<ImportState>>,
    group_name: String,
) -> Result<(), ServiceError> {
    let mut import_state = import_state.lock().await;
    import_state.add_new_import_set(Some(group_name));

    {
        let zip_file = File::open(path).await?;
        let buffered_reader = BufReader::new(zip_file);
        let mut archive = ZipFileReader::with_tokio(buffered_reader).await?;

        let len = archive.file().entries().len();

        for index in 0..len {
            let file = archive.reader_with_entry(index).await?;
            let link = import_state.origin_url.clone();

            if file.entry().dir()? {
                continue;
            }

            let path = PathBuf::from(file.entry().filename().as_str()?);

            if !is_supported_extension(&path) {
                continue;
            }

            let file_name = util::prettify_file_name(&path, false);
            let extension = path.extension().unwrap().to_str().unwrap();
            let file_size = usize::try_from(file.entry().uncompressed_size()).unwrap_or(0);
            let mut file_compat = file.compat();

            let id = import_single_model(
                &mut file_compat,
                extension,
                file_size,
                &file_name,
                link.as_deref(),
                app_state,
                &import_state.user,
                None,
            )
            .await?;

            import_state.add_model_id_to_current_set(id);
        }
    }

    if import_state.delete_after_import {
        let _ = fs::remove_file(path);
    }

    drop(import_state);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn import_single_model<W>(
    reader: &mut W,
    file_type: &str,
    file_size: usize,
    name: &str,
    link: Option<&str>,
    app_state: &AppState,
    user: &User,
    permanent_disk_path: Option<PathBuf>,
) -> Result<i64, ServiceError>
where
    W: AsyncRead + Unpin,
{
    let mut file_contents: Vec<u8> = Vec::with_capacity(file_size);

    reader.read_to_end(&mut file_contents).await?;

    let mut hasher = Sha256::new();
    hasher.update(&file_contents);
    let bytes = hasher.finalize();
    // First 128 bits of the digest as 32 hex chars via the shared `db::u128_to_hex_32` (same shape as `db::random_hex_32`).
    let hash = db::u128_to_hex_32(u128::from_be_bytes(
        bytes[..16].try_into().expect("slice is 16 bytes"),
    ));

    // Re-importing a trashed model restores it; this is also how sync propagates a restore.
    if let Some(id) = model_db::get_model_id_via_sha256(&app_state.db, user, &hash).await? {
        match model_db::untrash_models(&app_state.db, user, &[id]).await {
            Ok(()) => return Ok(id),
            // The trash purge deleted the expired model since the lookup, so import it afresh.
            // Its blob row is still there: the blob sweep waits for the import lock we hold.
            Err(db::DbError::RowNotFound) => {}
            Err(e) => return Err(e.into()),
        }
    }

    let blob_id = if let Some(blob) = blob_db::get_blob_via_sha256(&app_state.db, &hash).await? {
        blob.id
    } else if let Some(permanent_disk_path) = permanent_disk_path {
        // The blob stores the path as text, so a non-UTF-8 one can't be referenced.
        let disk_path = util::path_to_utf8(&permanent_disk_path)?;

        blob_db::add_blob(
            &app_state.db,
            &hash,
            file_type,
            i64::try_from(file_size).unwrap_or(i64::MAX),
            Some(disk_path.to_string()),
        )
        .await?
    } else {
        let new_extension = convert_extension_to_zip(file_type);

        let final_file_name = app_state
            .get_model_dir()
            .join(format!("{hash}.{new_extension}"));

        let mut file_handle = File::create(&final_file_name).await?;

        if FileType::from_extension(file_type).is_zippable() {
            let mut writer = ZipFileWriter::with_tokio(&mut file_handle);
            let builder = ZipEntryBuilder::new(
                format!("{name}.{}", file_type.to_lowercase()).into(),
                async_zip::Compression::Deflate,
            );

            writer.write_entry_whole(builder, &file_contents).await?;
            util::finish_zip(writer).await?;
        } else {
            file_handle.write_all(&file_contents).await?;
            // A tokio `File` can still have the write pending when `write_all` returns.
            file_handle.flush().await?;
        }

        blob_db::add_blob(
            &app_state.db,
            &hash,
            &new_extension,
            i64::try_from(file_size).unwrap_or(i64::MAX),
            None,
        )
        .await?
    };

    Ok(model_db::add_model(&app_state.db, user, name, blob_id, link, None).await?)
}

#[must_use]
pub fn is_supported_extension(path: &Path) -> bool {
    let file_type = FileType::from_pathbuf(path);
    file_type.is_importable()
}

/// Returns `true` if the path has a case-insensitive `.zip` extension.
fn is_zip_path(path: &Path) -> bool {
    libmeshthumbnail::path_ext::matches_ext(path, "zip")
}

/// Returns `true` for paths acceptable at an upload entry point.
///
/// That is a supported model extension or a zip archive. Shared by the web
/// and desktop upload handlers so the acceptance policy lives in one place.
#[must_use]
pub fn is_importable_upload(path: &Path) -> bool {
    is_supported_extension(path) || is_zip_path(path)
}

fn get_model_count_from_dir_recursive(path: &Path) -> Result<usize, ServiceError> {
    let entries: Vec<fs::DirEntry> = read_dir(path)?.collect::<Result<_, _>>()?;
    let mut count = 0;

    for folder in entries.iter().filter(|f| f.path().is_dir()) {
        // An unreadable subfolder counts as empty, since the recursive import skips it.
        count += get_model_count_from_dir_recursive(&folder.path()).unwrap_or(0);
    }

    count += get_model_count_from_dir(path)?;

    Ok(count)
}

fn get_model_count_from_dir(path: &Path) -> Result<usize, ServiceError> {
    let size = read_dir(path)?
        .map_ok(|entry| entry.path())
        .filter_ok(|path| path.is_file() && is_supported_extension(path))
        .process_results(|paths| paths.count())?;

    Ok(size)
}

async fn get_model_count_from_zip(path: &str) -> Result<usize, ServiceError> {
    let file = File::open(path).await?;
    let mut buffered_reader = BufReader::new(file);
    let zip = ZipFileReader::with_tokio(&mut buffered_reader).await?;
    let mut count = 0;

    for entry in zip.file().entries() {
        let path = PathBuf::from(entry.filename().as_str()?);

        if is_supported_extension(&path) {
            count += 1;
        }
    }

    Ok(count)
}

#[derive(Serialize)]
pub struct DirectoryScanModel {
    pub path: PathBuf,
    pub group_set: Option<u32>,
    pub group_name: Option<String>,
    pub model_ids: Option<Vec<i64>>,
}

async fn traverse_directory(
    path: &Path,
    recursive: bool,
    set_id: u32,
) -> Result<Vec<DirectoryScanModel>, ServiceError> {
    let mut models = Vec::new();

    let Ok(read_dir) = read_dir(path) else {
        return Ok(models);
    };

    let group_name = util::prettify_file_name(path, true);

    for entry in read_dir {
        let Ok(entry) = entry else { continue };

        let entry_path = entry.path();

        if entry_path.is_dir() && recursive {
            let mut sub_models =
                Box::pin(traverse_directory(&entry_path, recursive, set_id + 1)).await?;
            models.append(&mut sub_models);
        } else if entry_path.is_file() && is_supported_extension(&entry.path()) {
            models.push(DirectoryScanModel {
                path: entry.path(),
                group_set: Some(set_id),
                group_name: Some(group_name.clone()),
                model_ids: None,
            });
        }
    }

    Ok(models)
}

/// Expands paths (files/dirs) into a list of directory scan models.
pub async fn expand_paths(
    paths: &[PathBuf],
    recursive: bool,
) -> Result<Vec<DirectoryScanModel>, ServiceError> {
    let mut models = Vec::new();
    let mut set_id = 0;

    for path in paths {
        if path.is_dir() {
            let mut dir_models = traverse_directory(path, recursive, set_id).await?;
            models.append(&mut dir_models);
            set_id += 10000;
        } else if path.is_file() {
            models.push(DirectoryScanModel {
                path: path.clone(),
                group_set: None,
                group_name: None,
                model_ids: None,
            });
        }
    }

    Ok(models)
}

#[cfg(test)]
mod tests {
    use crate::service_error::ServiceError;

    use super::get_model_count;

    #[tokio::test]
    async fn get_model_count_unsupported_extension_returns_err() {
        let result = get_model_count("somefile.xyz", false, false).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        if let ServiceError::InternalError(msg) = &err {
            assert!(
                msg.contains("Unsupported file type"),
                "expected 'Unsupported file type' in message, got: {msg}"
            );
        } else {
            panic!("expected InternalError, got: {err:?}");
        }
    }

    #[tokio::test]
    async fn get_model_count_zip_with_import_as_path_returns_err() {
        let result = get_model_count("archive.zip", false, true).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        if let ServiceError::InternalError(msg) = &err {
            assert!(
                msg.contains("zip as path"),
                "expected 'zip as path' in message, got: {msg}"
            );
        } else {
            panic!("expected InternalError, got: {err:?}");
        }
    }
}
