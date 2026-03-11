use std::{
    env,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use chrono::Utc;
use regex::Regex;
use reqwest::{Response, header::CONTENT_DISPOSITION};
use serde::Serialize;
use urlencoding::decode;

use crate::{
    export_service::ensure_unique_file_full_filename, service_error::ServiceError,
    util::cleanse_evil_from_name,
};

#[derive(Serialize)]
pub struct DownloadResult {
    pub path: String,
    pub source_uri: Option<String>,
}

pub fn get_content_disposition_filename(response: &Response) -> Option<String> {
    let content_disposition = response.headers().get(CONTENT_DISPOSITION);

    match content_disposition {
        None => response.url().path().split("/").last().map(String::from),
        Some(header_value) => match header_value.to_str() {
            Ok(header_value) => {
                content_disposition::parse_content_disposition(header_value).filename_full()
            }
            Err(_) => None,
        },
    }
}

fn filename_from_response_or_url(response: &Response) -> String {
    get_content_disposition_filename(response)
        .or_else(|| {
            response
                .url()
                .path()
                .split('/')
                .next_back()
                .map(String::from)
        })
        .unwrap_or_else(|| "model.stl".to_string())
}

/// Downloads the resource at `url` into `dir` (caller-provided; no temp dir created).
/// Uses Content-Disposition or URL path segment for filename, cleanses it, writes to `dir`.
/// Returns the file path and the response URL (for callers that need redirect/site logic).
async fn download_file_to_dir(url: &str, dir: &Path) -> Result<(PathBuf, String), ServiceError> {
    let response = reqwest::get(url).await?;

    if !response.status().is_success() {
        return Err(ServiceError::InternalError(format!(
            "Failed to download file from url: {url}. Status code {}.",
            response.status()
        )));
    }

    let response_url = response.url().to_string();
    let filename = filename_from_response_or_url(&response);
    let cleansed = cleanse_evil_from_name(&filename);
    let file_path = ensure_unique_file_full_filename(dir, &cleansed);

    let mut file = File::create(&file_path)?;
    let bytes = response.bytes().await?;
    file.write_all(&bytes)?;

    Ok((file_path, response_url))
}

/// Simple download into a caller-provided directory. No site-specific filename/source_uri logic.
/// Returns `DownloadResult` with `path` set and `source_uri: None`.
pub async fn download_file_to(url: &str, dir: &Path) -> Result<DownloadResult, ServiceError> {
    let (path, _) = download_file_to_dir(url, dir).await?;
    let path_str = path
        .to_str()
        .ok_or_else(|| ServiceError::InternalError("Path is not valid UTF-8".into()))?;
    Ok(DownloadResult {
        path: path_str.to_string(),
        source_uri: None,
    })
}

pub async fn download_file(url: &str) -> Result<DownloadResult, ServiceError> {
    let temp_dir = env::temp_dir().join(format!(
        "meshorganiser_download_action_{}",
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    fs::create_dir_all(&temp_dir)?;

    let (mut file_path, response_url) = download_file_to_dir(url, &temp_dir).await?;

    let redirect_url_filename = match response_url.split('/').next_back() {
        Some(seg) => decode(seg).unwrap_or_default().into_owned(),
        None => "model.stl".to_string(),
    };

    let mut source_uri: Option<String> = None;
    let desired_filename: String = if url.contains("makerworld") {
        source_uri = Some(String::from("https://makerworld.com"));
        url.split("name=").last().unwrap_or("model.stl").to_string()
    } else if url.contains("thingiverse") {
        source_uri = Some(String::from("https://www.thingiverse.com/"));
        redirect_url_filename
    } else if let Some(stripped) = url.strip_prefix("https://files.printables.com/media/prints/") {
        let id = String::from(stripped.split('/').next().unwrap());
        source_uri = Some(format!("https://www.printables.com/model/{id}"));
        file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("model.stl")
            .to_string()
    } else if url.contains("nexprint") {
        source_uri = Some(String::from("https://www.nexprint.com/"));
        let re = Regex::new(r#"filename="([^"]+)""#).unwrap();
        let decoded_url = decode(url).unwrap().into_owned();
        re.captures(&decoded_url)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| {
                file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("model.stl")
                    .to_string()
            })
    } else {
        file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("model.stl")
            .to_string()
    };

    let cleansed_desired = cleanse_evil_from_name(&desired_filename);
    let current_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if cleansed_desired != current_name {
        let new_path = ensure_unique_file_full_filename(&temp_dir, &cleansed_desired);
        fs::rename(&file_path, &new_path)?;
        file_path = new_path;
    }

    let path = file_path
        .to_str()
        .ok_or_else(|| ServiceError::InternalError("Path is not valid UTF-8".into()))?
        .to_string();

    Ok(DownloadResult { path, source_uri })
}
