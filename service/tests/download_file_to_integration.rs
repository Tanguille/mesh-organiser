//! Integration tests for `download_file_to` using a mock HTTP server.
//!
//! We test against a local mock server (wiremock) instead of real URLs to avoid
//! flakiness, network dependency, and external service changes. This gives
//! regression coverage so that consolidating download logic in the service
//! crate does not break behaviour.

use std::path::PathBuf;

use service::download_file_service::{download_file_to, get_content_disposition_filename};
use tempfile::tempdir;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};

#[tokio::test]
async fn download_file_to_writes_file_with_content_disposition() {
    let mock_server = MockServer::start().await;

    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("test content")
                .insert_header(
                    "Content-Disposition",
                    r#"attachment; filename="example.stl""#,
                ),
        )
        .mount(&mock_server)
        .await;

    let dir = tempdir().unwrap();
    let url = format!("{base}/file", base = mock_server.uri());
    let result = download_file_to(&url, dir.path())
        .await
        .expect("download should succeed");

    let path = PathBuf::from(&result.path);
    assert!(
        path.exists(),
        "downloaded file should exist at {}",
        result.path
    );
    assert!(
        path.file_name().is_some_and(|n| n == "example.stl"),
        "filename should be example.stl from Content-Disposition, got {:?}",
        path.file_name()
    );
    let contents = std::fs::read_to_string(&path).expect("read file");
    assert_eq!(contents, "test content");
}

#[tokio::test]
async fn download_file_to_uses_url_path_when_no_content_disposition() {
    let mock_server = MockServer::start().await;

    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string("test content"))
        .mount(&mock_server)
        .await;

    let dir = tempdir().unwrap();
    let url = format!("{base}/some/path/model.stl", base = mock_server.uri());
    let result = download_file_to(&url, dir.path())
        .await
        .expect("download should succeed");

    let path = PathBuf::from(&result.path);
    assert!(path.exists());
    assert!(
        path.file_name().is_some_and(|n| n == "model.stl"),
        "filename should come from URL path, got {:?}",
        path.file_name()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "test content");
}

#[tokio::test]
async fn get_content_disposition_filename_uses_header_when_present() {
    let mock_server = MockServer::start().await;

    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("x")
                .insert_header(
                    "Content-Disposition",
                    r#"attachment; filename="from-header.stl""#,
                ),
        )
        .mount(&mock_server)
        .await;

    let url = format!("{base}/any/path/ignored.stl", base = mock_server.uri());
    let response = reqwest::get(&url).await.expect("GET should succeed");
    let name = get_content_disposition_filename(&response);
    assert_eq!(name.as_deref(), Some("from-header.stl"));
}

#[tokio::test]
async fn get_content_disposition_filename_uses_url_path_when_header_missing() {
    let mock_server = MockServer::start().await;

    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string("x"))
        .mount(&mock_server)
        .await;

    let url = format!("{base}/some/path/model.stl", base = mock_server.uri());
    let response = reqwest::get(&url).await.expect("GET should succeed");
    let name = get_content_disposition_filename(&response);
    assert_eq!(name.as_deref(), Some("model.stl"));
}

#[tokio::test]
async fn get_content_disposition_filename_rfc5987_from_header() {
    let mock_server = MockServer::start().await;

    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("x")
                .insert_header(
                    "Content-Disposition",
                    r"attachment; filename*=UTF-8''my%20percent%20file.stl",
                ),
        )
        .mount(&mock_server)
        .await;

    let url = format!("{base}/url.stl", base = mock_server.uri());
    let response = reqwest::get(&url).await.expect("GET should succeed");
    let name = get_content_disposition_filename(&response);
    assert_eq!(name.as_deref(), Some("my percent file.stl"));
}
