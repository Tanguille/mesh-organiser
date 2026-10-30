use std::{
    collections::HashMap,
    panic,
    path::{Path, PathBuf},
};

use async_zip::{Compression, ZipEntryBuilder, tokio::write::ZipFileWriter};
use futures::future::try_join_all;
use serde::Serialize;
use tauri::{AppHandle, State, http::header::CONTENT_TYPE};
use tauri_plugin_http::reqwest;
use tokio::{fs::File, io::BufReader, task::JoinSet};
use tokio_util::{compat::TokioAsyncReadCompatExt, io::ReaderStream};

use crate::{error::ApplicationError, tauri_app_state::TauriAppState, tauri_import_state};

use service::{
    ServiceError, download_file_service,
    export_service::get_temp_dir,
    import_service::DirectoryScanModel,
    import_state::{ImportState, ImportStatus},
};

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
    let results = try_join_all(
        urls.iter()
            .map(|url| download_file_service::download_file_to(url, &temp_dir)),
    )
    .await?;
    let paths = results
        .into_iter()
        .map(|result| PathBuf::from(result.path))
        .collect();

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

        service::finish_zip(writer).await?;
    }

    tauri_plugin_opener::open_path(&temp_dir, None::<&str>)
        .map_err(|error| ServiceError::InternalError(error.to_string()))?;

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
        slicer.open(temp_dir.1, &state.app_state)?;
    }

    Ok(())
}

async fn login(token: &str, base_url: &str) -> Result<reqwest::Client, ApplicationError> {
    let client = reqwest::ClientBuilder::new()
        .cookie_store(true)
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
        return Err(ServiceError::InternalError("Failed to login with token".into()).into());
    }

    Ok(client)
}

async fn logout(client: &reqwest::Client, base_url: &str) -> Result<(), ApplicationError> {
    let url = format!("{base_url}/api/v1/logout");

    let response = client.post(&url).send().await?;

    if !response.status().is_success() {
        return Err(ServiceError::InternalError("Failed to logout".into()).into());
    }

    Ok(())
}

const MAX_CONCURRENT_UPLOADS: usize = 4;

/// Streams a single model file to the remote server as a multipart upload.
/// Opening and streaming inside the upload task keeps at most the in-flight
/// chunks in memory instead of whole files.
async fn upload_model_file(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    source_url: Option<String>,
) -> Result<reqwest::Response, ApplicationError> {
    let mut form = reqwest::multipart::Form::new();

    if let Some(source_url) = source_url {
        form = form.text("source_url", source_url);
    }

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let file = File::open(path).await?;
    let len = file.metadata().await?.len();
    // stream_with_length sends a Content-Length header instead of switching to
    // chunked transfer.
    let part = reqwest::multipart::Part::stream_with_length(
        reqwest::Body::wrap_stream(ReaderStream::new(file)),
        len,
    )
    .file_name(filename)
    .mime_str("application/octet-stream")?;
    form = form.part("file", part);

    Ok(client.post(url).multipart(form).send().await?)
}

async fn get_ids(response: reqwest::Response) -> Result<Vec<i64>, ApplicationError> {
    if response.status().is_success() {
        let body = response.text().await?;
        let model_ids: Vec<i64> = serde_json::from_str(&body)?;

        if model_ids.is_empty() {
            println!("Upload returned no model IDs");
            return Err(
                ServiceError::InternalError("No model IDs returned after upload".into()).into(),
            );
        }

        Ok(model_ids)
    } else {
        let err = format!(
            "Upload failed with status: {} and response '{}'",
            response.status(),
            response.text().await.unwrap_or_default()
        );
        println!("{err}");
        Err(ServiceError::InternalError(err).into())
    }
}

async fn process_uploads(
    client: &reqwest::Client,
    base_url: &str,
    paths: &mut Vec<DirectoryScanModel>,
    mut import_state: ImportState,
) -> Result<ImportState, ApplicationError> {
    import_state.update_status(ImportStatus::ProcessingModels);
    import_state.update_total_model_count(paths.len());

    let url = format!("{base_url}/api/v1/models");
    let mut futures = JoinSet::new();

    let mut results = Vec::new();

    for path in &mut *paths {
        if !path.path.exists() {
            println!(
                "Warning: Path {} does not exist, skipping upload",
                path.path.display()
            );
            import_state.update_total_model_count(import_state.model_count - 1);
            continue;
        }

        {
            let path = PathBuf::from(&path.path);
            let client = client.clone();
            let url = url.clone();
            let source_url = import_state.origin_url.clone();
            futures.spawn(async move {
                let result = upload_model_file(&client, &url, &path, source_url).await;
                (path, result)
            });
        }

        if futures.len() >= MAX_CONCURRENT_UPLOADS
            && let Some(res) = futures.join_next().await
        {
            match res {
                Err(err) if err.is_panic() => panic::resume_unwind(err.into_panic()),
                Err(err) => {
                    return Err(
                        ServiceError::InternalError(format!("Upload task failed: {err}")).into(),
                    );
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
        for model_id in &ids {
            import_state.add_model_id_to_current_set(*model_id);
        }
        import_state.update_total_model_count(import_state.model_count + ids.len() - 1);
        results.push((path, ids));
    }

    // Index results by path so matching back to the scan list is linear.
    let mut results: HashMap<PathBuf, Vec<i64>> = results.into_iter().collect();
    for path in paths.iter_mut() {
        path.model_ids = results.remove(&path.path);
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
        return Err(ServiceError::InternalError("No sync URL set for user".into()).into());
    };
    let Some(token) = user.sync_token else {
        return Err(ServiceError::InternalError("No sync token set for user".into()).into());
    };
    let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();

    let client = login(&token, &base_url).await?;
    let mut scan = service::import_service::expand_paths(&paths, recursive).await?;
    let import_state = tauri_import_state::import_state_new_tauri(
        source_url,
        false,
        false,
        false,
        &app_state,
        &app_handle,
    );
    let import_state = process_uploads(&client, &base_url, &mut scan, import_state).await?;

    logout(&client, &base_url).await?;

    if open_in_slicer
        && !scan.is_empty()
        && let Some(slicer) = &app_state.get_configuration().slicer
    {
        let model_paths: Vec<PathBuf> = scan.iter().map(|m| m.path.clone()).collect();
        slicer.open(model_paths, &app_state.app_state)?;
    }

    Ok(UploadResult {
        import_state,
        uploaded_models: scan,
    })
}

#[cfg(test)]
mod tests {
    struct UploadDirectory(std::path::PathBuf);

    impl Drop for UploadDirectory {
        fn drop(&mut self) {
            if let Err(e) = std::fs::remove_dir_all(&self.0) {
                eprintln!("Failed to remove upload test directory {:?}: {e}", self.0);
            }
        }
    }

    #[test]
    fn upload_directory_cleanup_tolerates_an_already_removed_directory() {
        let directory = UploadDirectory(super::get_temp_dir("upload-cleanup-test"));
        std::fs::remove_dir_all(&directory.0).unwrap();

        drop(directory);
    }

    fn upload_server(upload_count: usize) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let timeout = std::time::Duration::from_secs(5);
            for index in 0..upload_count {
                let started = std::time::Instant::now();
                let (stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && started.elapsed() < timeout =>
                        {
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err(e) => panic!("mock upload server failed to accept: {e}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(timeout)).unwrap();
                stream.set_write_timeout(Some(timeout)).unwrap();
                let mut reader = std::io::BufReader::new(stream);
                let mut line = String::new();
                std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
                assert!(line.starts_with("POST /api/v1/models HTTP/1.1"));
                let mut content_length = None;
                loop {
                    line.clear();
                    assert!(std::io::BufRead::read_line(&mut reader, &mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(length) = line.to_ascii_lowercase().strip_prefix("content-length:")
                    {
                        content_length = Some(length.trim().parse::<usize>().unwrap());
                    }
                }
                let mut body = vec![0; content_length.unwrap()];
                std::io::Read::read_exact(&mut reader, &mut body).unwrap();
                let first_id = index * 2 + 1;
                let second_id = first_id + 1;
                let body = format!("[{first_id},{second_id}]");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                std::io::Write::write_all(reader.get_mut(), response.as_bytes()).unwrap();
            }
        });

        (base_url, server)
    }

    #[tokio::test]
    async fn process_uploads_counts_only_uploaded_models_including_drained_tasks() {
        for (existing_count, missing_count) in [(0, 0), (0, 2), (1, 1), (5, 1)] {
            let directory = UploadDirectory(super::get_temp_dir("upload-test"));
            let mut paths = Vec::new();
            for index in 0..existing_count + missing_count {
                let path = directory.0.join(format!("model-{index}.stl"));
                if index < existing_count {
                    std::fs::write(&path, b"test model").unwrap();
                }
                paths.push(super::DirectoryScanModel {
                    path,
                    group_set: None,
                    group_name: None,
                    model_ids: None,
                });
            }

            let (base_url, server) = upload_server(existing_count);

            let state = service::import_state::ImportState::new(
                None,
                false,
                false,
                false,
                db::model::user::User::default(),
            );
            let client = super::reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap();
            let result = super::process_uploads(&client, &base_url, &mut paths, state)
                .await
                .unwrap();
            server.join().unwrap();

            assert_eq!(result.model_count, existing_count * 2);
            assert_eq!(result.imported_model_count, existing_count * 2);
            assert!(matches!(result.status, super::ImportStatus::Finished));
            let mut imported_ids = result.all_model_ids();
            imported_ids.sort_unstable();
            let expected_ids: Vec<i64> = (1..=i64::try_from(existing_count * 2).unwrap()).collect();
            assert_eq!(imported_ids, expected_ids);
            let mut mapped_ids = Vec::new();
            for (index, path) in paths.iter().enumerate() {
                if index < existing_count {
                    let ids = path.model_ids.as_ref().unwrap();
                    assert_eq!(ids.len(), 2);
                    mapped_ids.extend_from_slice(ids);
                } else {
                    assert!(path.model_ids.is_none());
                }
            }
            mapped_ids.sort_unstable();
            assert_eq!(mapped_ids, expected_ids);
        }
    }
}
