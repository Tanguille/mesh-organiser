use std::{
    env,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
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

static FILENAME_QUOTED: OnceLock<Regex> = OnceLock::new();
static FILENAME_UNQUOTED: OnceLock<Regex> = OnceLock::new();

#[derive(Serialize)]
pub struct DownloadResult {
    pub path: String,
    pub source_uri: Option<String>,
}

/// Parses a Content-Disposition header value and returns the filename if present.
/// Supports `filename*=UTF-8''<percent-encoded>` (RFC 5987) and `filename="..."` / `filename=value`.
fn parse_content_disposition_filename(header_value: &str) -> Option<String> {
    // Prefer filename*=UTF-8''<encoded> (RFC 5987)
    if let Some(rest) = header_value.split("filename*=").nth(1) {
        let token = rest.split(';').next().unwrap_or(rest).trim();
        if let Some(encoded) = token
            .strip_prefix("UTF-8''")
            .or_else(|| token.strip_prefix("utf-8''"))
            && let Ok(decoded) = decode(encoded)
        {
            return Some(decoded.into_owned());
        }
    }
    // Fallback: filename="..." or filename=value
    let re = FILENAME_QUOTED.get_or_init(|| Regex::new(r#"filename\s*=\s*"([^"]*)""#).unwrap());
    if let Some(cap) = re.captures(header_value)
        && let Some(m) = cap.get(1)
    {
        return Some(m.as_str().to_string());
    }
    let re2 = FILENAME_UNQUOTED.get_or_init(|| Regex::new(r"filename\s*=\s*([^;\s]+)").unwrap());
    if let Some(cap) = re2.captures(header_value)
        && let Some(m) = cap.get(1)
    {
        return Some(m.as_str().trim().to_string());
    }

    None
}

pub fn get_content_disposition_filename(response: &Response) -> Option<String> {
    response.headers().get(CONTENT_DISPOSITION).map_or_else(
        || {
            response
                .url()
                .path()
                .split('/')
                .next_back()
                .map(String::from)
        },
        |header_value| {
            header_value
                .to_str()
                .ok()
                .and_then(parse_content_disposition_filename)
        },
    )
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

static MAKERWORLD_MODEL_PAGE: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();

/// If either URL contains a `MakerWorld` model page path (`/{locale}/models/{slug}` or `/models/{slug}`),
/// returns that page URL substring from the haystack. Otherwise returns `None` for caller fallback.
///
/// If the built-in pattern fails to compile, returns `None` on every call so the download path uses
/// the generic `MakerWorld` site URL instead of panicking.
fn makerworld_model_page_url(original_url: &str, response_url: &str) -> Option<String> {
    let regex = MAKERWORLD_MODEL_PAGE
        .get_or_init(|| {
            Regex::new(concat!(
                r"(?i)https://(?:www\.)?makerworld\.com(?:\.cn)?",
                r"(?:/(?:[a-z]{2}(?:-[a-z]{2})?))?/models/[\w-]+",
            ))
        })
        .as_ref()
        .ok()?;

    regex
        .find(original_url)
        .or_else(|| regex.find(response_url))
        .map(|m| m.as_str().to_string())
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

/// Simple download into a caller-provided directory. No site-specific `filename`/`source_uri` logic.
/// Returns `DownloadResult` with `path` set and `source_uri: None`.
///
/// # Errors
///
/// Returns an error if the request fails, the response is not successful, or file I/O fails.
pub async fn download_file_to(url: &str, dir: &Path) -> Result<DownloadResult, ServiceError> {
    let (path, _) = download_file_to_dir(url, dir).await?;
    let path = path
        .to_str()
        .ok_or_else(|| ServiceError::InternalError("Path is not valid UTF-8".into()))?;
    Ok(DownloadResult {
        path: path.to_string(),
        source_uri: None,
    })
}

/// Downloads to a temp dir and derives filename/source from URL; may rename file for site-specific logic.
///
/// # Errors
///
/// Returns an error if the request fails, the response is not successful, or file I/O fails.
///
/// # Panics
///
/// Panics if the system clock cannot provide nanosecond timestamps for the temp dir name.
pub async fn download_file(url: &str) -> Result<DownloadResult, ServiceError> {
    let temp_dir = env::temp_dir().join(format!(
        "meshorganiser_download_action_{}",
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    fs::create_dir_all(&temp_dir)?;

    let (mut file_path, response_url) = download_file_to_dir(url, &temp_dir).await?;

    let redirect_url_filename = response_url.split('/').next_back().map_or_else(
        || "model.stl".to_string(),
        |seg| decode(seg).unwrap_or_default().into_owned(),
    );

    let mut source_uri: Option<String> = None;
    let desired_filename: String = if url.contains("makerworld") {
        source_uri = Some(
            makerworld_model_page_url(url, &response_url)
                .unwrap_or_else(|| String::from("https://makerworld.com/")),
        );
        url.split("name=").last().unwrap_or("model.stl").to_string()
    } else if url.contains("thingiverse") {
        source_uri = Some(String::from("https://thingiverse.com/"));
        redirect_url_filename
    } else if let Some(stripped) = url.strip_prefix("https://files.printables.com/media/prints/") {
        let id = String::from(stripped.split('/').next().unwrap());
        source_uri = Some(format!("https://printables.com/model/{id}"));
        file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("model.stl")
            .to_string()
    } else if url.contains("nexprint") {
        source_uri = Some(String::from("https://nexprint.com/"));
        let re = Regex::new(r#"filename="([^"]+)""#).unwrap();
        let decoded_url = decode(url).unwrap().into_owned();
        re.captures(&decoded_url)
            .and_then(|c| c.get(1))
            .map_or_else(
                || {
                    file_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("model.stl")
                        .to_string()
                },
                |m| m.as_str().to_string(),
            )
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

#[cfg(test)]
mod tests {
    use super::{makerworld_model_page_url, parse_content_disposition_filename};

    #[test]
    fn parse_content_disposition_filename_rfc5987_utf8_percent_encoded() {
        // RFC 5987: filename*=UTF-8''<percent-encoded>
        assert_eq!(
            parse_content_disposition_filename(r"attachment; filename*=UTF-8''my%20file.stl"),
            Some("my file.stl".to_string())
        );
        assert_eq!(
            parse_content_disposition_filename(r"filename*=UTF-8''%E4%B8%AD%E6%96%87.stl"),
            Some("中文.stl".to_string())
        );
    }

    #[test]
    fn parse_content_disposition_filename_rfc5987_charset_case_insensitive() {
        // Charset is case-insensitive (utf-8'' vs UTF-8'')
        assert_eq!(
            parse_content_disposition_filename(r"attachment; filename*=utf-8''file.stl"),
            Some("file.stl".to_string())
        );
    }

    #[test]
    fn parse_content_disposition_filename_quoted() {
        assert_eq!(
            parse_content_disposition_filename(r#"attachment; filename="something.stl""#),
            Some("something.stl".to_string())
        );
        assert_eq!(
            parse_content_disposition_filename(r#"inline; filename="with spaces.stl""#),
            Some("with spaces.stl".to_string())
        );
    }

    #[test]
    fn parse_content_disposition_filename_unquoted() {
        assert_eq!(
            parse_content_disposition_filename("attachment; filename=unquoted.stl"),
            Some("unquoted.stl".to_string())
        );
        assert_eq!(
            parse_content_disposition_filename("filename=token"),
            Some("token".to_string())
        );
    }

    #[test]
    fn parse_content_disposition_filename_empty_quoted() {
        // Empty quoted filename="" — unquoted branch matches first and captures the quote(s)
        assert_eq!(
            parse_content_disposition_filename(r#"filename=""#),
            Some("\"".to_string())
        );
    }

    #[test]
    fn parse_content_disposition_filename_missing_or_malformed_returns_none() {
        assert_eq!(parse_content_disposition_filename("attachment"), None);
        assert_eq!(parse_content_disposition_filename(""), None);
        assert_eq!(parse_content_disposition_filename("filename="), None);
        // filename*= with wrong charset / not UTF-8''
        assert_eq!(
            parse_content_disposition_filename(r"filename*=ISO-8859-1''%20"),
            None
        );
    }

    #[test]
    fn makerworld_model_page_url_extracts_from_request_url() {
        assert_eq!(
            makerworld_model_page_url(
                "https://makerworld.com/en/models/1866618-wheel-loader-kit-card?download=1",
                "https://cdn.example.com/redirected",
            ),
            Some("https://makerworld.com/en/models/1866618-wheel-loader-kit-card".to_string())
        );
    }

    #[test]
    fn makerworld_model_page_url_extracts_from_response_when_request_has_no_model_path() {
        assert_eq!(
            makerworld_model_page_url(
                "https://files.makerworld.com/foo?bar=1",
                "https://www.makerworld.com/de/models/42-name/stuff",
            ),
            Some("https://www.makerworld.com/de/models/42-name".to_string())
        );
    }

    #[test]
    fn makerworld_model_page_url_none_when_no_model_segment() {
        assert_eq!(
            makerworld_model_page_url(
                "https://signed.cdn.example.com/file?X-Amz-Signature=abc",
                "https://other.example.net/model.stl",
            ),
            None
        );
    }

    #[test]
    fn makerworld_model_page_url_prefers_original_url_when_both_contain_model_path() {
        assert_eq!(
            makerworld_model_page_url(
                "https://makerworld.com/en/models/original-slug",
                "https://www.makerworld.com/de/models/other-slug",
            ),
            Some("https://makerworld.com/en/models/original-slug".to_string())
        );
    }

    #[test]
    fn makerworld_model_page_url_extracts_zh_cn_locale() {
        assert_eq!(
            makerworld_model_page_url(
                "https://makerworld.com/zh-cn/models/123-slug",
                "https://cdn.example.com/",
            ),
            Some("https://makerworld.com/zh-cn/models/123-slug".to_string())
        );
    }

    #[test]
    fn makerworld_model_page_url_extracts_com_cn_host() {
        assert_eq!(
            makerworld_model_page_url(
                "https://makerworld.com.cn/en/models/1-slug",
                "https://x.example/",
            ),
            Some("https://makerworld.com.cn/en/models/1-slug".to_string())
        );
    }

    #[test]
    fn makerworld_model_page_url_case_insensitive_match() {
        assert_eq!(
            makerworld_model_page_url(
                "HTTPS://WWW.MAKERWORLD.COM/EN/models/999-ABC",
                "https://ignore.example/",
            ),
            Some("HTTPS://WWW.MAKERWORLD.COM/EN/models/999-ABC".to_string())
        );
    }
}
