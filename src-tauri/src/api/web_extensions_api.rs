use std::{
    panic,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_zip::{tokio::write::ZipFileWriter, Compression, ZipEntryBuilder};
use futures::future::try_join_all;
use serde::Serialize;
use tauri::{http::header::CONTENT_TYPE, ipc::Response, AppHandle, State};
use tauri_plugin_http::reqwest::{self, cookie::Jar};
use tokio::{fs::File, io::BufReader, task::JoinSet};
use tokio_util::compat::TokioAsyncReadCompatExt;

use crate::{error::ApplicationError, tauri_app_state::TauriAppState, tauri_import_state};

use service::{
    download_file_service,
    export_service::get_temp_dir,
    import_service::{self, is_supported_extension, DirectoryScanModel},
    import_state::{ImportState, ImportStatus},
};

async fn download_file(url: &str, dir: &Path) -> Result<PathBuf, ApplicationError> {
    let result = download_file_service::download_file_to(url, dir)
        .await
        .map_err(ApplicationError::from)?;
    Ok(PathBuf::from(result.path))
}

async fn download_files_to_temp_dir(
    sha256s: Vec<String>,
    base_url: &str,
    user_id: i64,
    user_hash: &str,
) -> Result<(PathBuf, Vec<PathBuf>), ApplicationError> {
    let temp_dir = get_temp_dir("download");
    let urls: Vec<String> = sha256s
        .iter()
        .map(|sha256| {
            format!(
                "{base_url}/api/v1/blobs/{sha256}/download?user_id={user_id}&user_hash={user_hash}"
            )
        })
        .collect();
    let download_futures: Vec<_> = urls
        .iter()
        .map(|url| download_file(url, &temp_dir))
        .collect();
    let paths = try_join_all(download_futures).await?;
    Ok((temp_dir, paths))
}

#[tauri::command]
pub async fn download_files_and_open_in_folder(
    sha256s: Vec<String>,
    base_url: &str,
    user_id: i64,
    user_hash: &str,
    as_zip: bool,
) -> Result<(), ApplicationError> {
    let (temp_dir, model_paths) =
        download_files_to_temp_dir(sha256s, base_url, user_id, user_hash).await?;

    if as_zip {
        const COPY_BUF_SIZE: usize = 256 * 1024; // 256 KiB for fewer syscalls
        let zip_path = temp_dir.join("export.zip");
        let mut file = File::create(&zip_path).await?;
        let mut writer = ZipFileWriter::with_tokio(&mut file);

        for model_path in model_paths {
            let file_name = model_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let builder = ZipEntryBuilder::new(file_name.into(), Compression::Deflate);
            let mut stream_writer = writer.write_entry_stream(builder).await?;

            let model_file = File::open(&model_path).await?;
            let mut reader = BufReader::with_capacity(COPY_BUF_SIZE, model_file).compat();
            futures::io::copy(&mut reader, &mut stream_writer).await?;
            stream_writer.close().await?;
            drop(reader);
            let _ = std::fs::remove_file(&model_path);
        }

        writer.close().await?;
    }

    service::open_folder_in_explorer(&temp_dir);

    Ok(())
}

#[tauri::command]
pub async fn download_files_and_open_in_slicer(
    sha256s: Vec<String>,
    base_url: &str,
    user_id: i64,
    user_hash: &str,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    if let Some(slicer) = &state.get_configuration().slicer {
        let temp_dir = download_files_to_temp_dir(sha256s, base_url, user_id, user_hash).await?;
        slicer.open(temp_dir.1, &state.app_state).await?;
    }

    Ok(())
}

async fn login(token: &str, base_url: &str) -> Result<Arc<Jar>, ApplicationError> {
    let jar = Arc::new(Jar::default());
    let client = reqwest::ClientBuilder::new()
        .cookie_provider(Arc::clone(&jar))
        .build()
        .unwrap();

    let url = format!("{base_url}/api/v1/login/token");

    let body = serde_json::to_vec(&serde_json::json!({ "token": token }))?;
    let response = client
        .post(&url)
        .header(CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(ApplicationError::InternalError(
            "Failed to login with token".into(),
        ));
    }

    Ok(jar)
}

async fn logout(jar: Arc<Jar>, base_url: &str) -> Result<(), ApplicationError> {
    let client = reqwest::ClientBuilder::new()
        .cookie_provider(Arc::clone(&jar))
        .build()
        .unwrap();

    let url = format!("{base_url}/api/v1/logout");

    let response = client.post(&url).send().await?;

    if !response.status().is_success() {
        return Err(ApplicationError::InternalError("Failed to logout".into()));
    }

    Ok(())
}

const MAX_CONCURRENT_UPLOADS: usize = 4;

async fn get_ids(response: reqwest::Response) -> Result<Vec<i64>, ApplicationError> {
    if response.status().is_success() {
        let body = response.text().await?;
        let model_ids: Vec<i64> = serde_json::from_str(&body)?;

        if model_ids.is_empty() {
            println!("Upload returned no model IDs");
            return Err(ApplicationError::InternalError(
                "No model IDs returned after upload".into(),
            ));
        }

        Ok(model_ids)
    } else {
        let err = format!(
            "Upload failed with status: {} and response '{}'",
            response.status(),
            response.text().await.unwrap_or_default()
        );
        println!("{err}");
        Err(ApplicationError::InternalError(err))
    }
}

async fn process_uploads(
    jar: Arc<Jar>,
    base_url: &str,
    paths: &mut Vec<DirectoryScanModel>,
    source_url: Option<String>,
    app_state: &TauriAppState,
    app_handle: &AppHandle,
) -> Result<ImportState, ApplicationError> {
    let mut import_state = tauri_import_state::import_state_new_tauri(
        source_url, false, false, false, app_state, app_handle,
    );
    import_state.update_status(ImportStatus::ProcessingModels);
    import_state.update_total_model_count(paths.len());

    let client = reqwest::ClientBuilder::new()
        .cookie_provider(Arc::clone(&jar))
        .build()
        .unwrap();

    let url = format!("{base_url}/api/v1/models");
    let mut futures = JoinSet::new();

    let mut results = Vec::new();

    for path in &mut *paths {
        let mut form = reqwest::multipart::Form::new();

        if let Some(source_url) = &import_state.origin_url {
            form = form.text("source_url", source_url.clone());
        }

        if !path.path.exists() {
            println!(
                "Warning: Path {} does not exist, skipping upload",
                path.path.display()
            );
        }

        let file_bytes = tokio::fs::read(&path.path).await?;
        let filename = path
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        let part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(filename.to_string())
            .mime_str("application/octet-stream")?;
        form = form.part("file", part);

        {
            let path = PathBuf::from(&path.path);
            let client = client.clone();
            let url = url.clone();
            futures.spawn(async move { (path, client.post(&url).multipart(form).send().await) });
        }

        if futures.len() >= MAX_CONCURRENT_UPLOADS
            && let Some(res) = futures.join_next().await
        {
            match res {
                Err(err) if err.is_panic() => panic::resume_unwind(err.into_panic()),
                Err(err) => {
                    return Err(ApplicationError::InternalError(format!(
                        "Upload task failed: {err}"
                    )));
                }
                Ok(response) => {
                    let path = response.0;
                    let response = response.1?;
                    let ids = get_ids(response).await?;

                    for model_id in &ids {
                        import_state.add_model_id_to_current_set(*model_id);
                    }

                    import_state.update_total_model_count(import_state.model_count + ids.len() - 1);

                    results.push((path, ids));
                }
            }
        }
    }

    for response in futures.join_all().await {
        let path = response.0;
        let response = response.1?;
        let ids = get_ids(response).await?;
        results.push((path, ids));
    }

    for result in results {
        let path = result.0;
        let model_ids = result.1;

        // Not super efficient, fix later
        let path = paths.iter_mut().find(|p| p.path == path).unwrap();

        path.model_ids = Some(model_ids);
    }

    import_state.update_status(ImportStatus::Finished);
    Ok(import_state)
}

#[derive(Serialize)]
pub struct UploadResult {
    pub import_state: ImportState,
    pub uploaded_models: Vec<DirectoryScanModel>,
}

#[tauri::command]
pub async fn upload_models_to_remote_server(
    paths: Vec<String>,
    source_url: Option<String>,
    recursive: bool,
    open_in_slicer: bool,
    app_state: State<'_, TauriAppState>,
    app_handle: AppHandle,
) -> Result<UploadResult, ApplicationError> {
    let user = app_state.get_current_user();
    let Some(base_url) = user.sync_url else {
        return Err(ApplicationError::InternalError(
            "No sync URL set for user".into(),
        ));
    };
    let Some(token) = user.sync_token else {
        return Err(ApplicationError::InternalError(
            "No sync token set for user".into(),
        ));
    };
    let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();

    let jar = login(&token, &base_url).await?;
    let mut scan = import_service::expand_paths(&paths, recursive).await?;
    let import_state = process_uploads(
        Arc::clone(&jar),
        &base_url,
        &mut scan,
        source_url,
        &app_state,
        &app_handle,
    )
    .await?;

    logout(jar, &base_url).await?;

    if open_in_slicer
        && !scan.is_empty()
        && let Some(slicer) = &app_state.get_configuration().slicer
    {
        let model_paths: Vec<PathBuf> = scan.iter().map(|m| m.path.clone()).collect();
        slicer.open(model_paths, &app_state.app_state).await?;
    }

    Ok(UploadResult {
        import_state,
        uploaded_models: scan,
    })
}

#[tauri::command]
pub async fn expand_paths(
    paths: Vec<String>,
    recursive: bool,
) -> Result<Vec<DirectoryScanModel>, ApplicationError> {
    let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();

    Ok(import_service::expand_paths(&paths, recursive).await?)
}

#[tauri::command]
pub async fn get_file_bytes(path: String) -> Result<Response, ApplicationError> {
    let path = PathBuf::from(path);

    if !(is_supported_extension(&path)
        || path
            .extension()
            .is_some_and(|e| e.to_string_lossy().to_lowercase().ends_with("zip")))
    {
        return Err(ApplicationError::InternalError(
            "Unsupported file extension for getting bytes".into(),
        ));
    }

    let bytes = tokio::fs::read(&path).await?;

    Ok(Response::new(bytes))
}
