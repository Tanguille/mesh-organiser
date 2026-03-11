//! Integration tests for `download_file_to` using a mock HTTP server.
//!
//! We test against a local mock server (wiremock) instead of real URLs to avoid
//! flakiness, network dependency, and external service changes. This gives
//! regression coverage so that consolidating download logic in the service
//! crate does not break behaviour.

use std::path::PathBuf;

use service::download_file_service::download_file_to;
use tempfile::tempdir;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

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
    let url = format!("{}/file", mock_server.uri());
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
        path.file_name()
            .map(|n| n == "example.stl")
            .unwrap_or(false),
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
    let url = format!("{}/some/path/model.stl", mock_server.uri());
    let result = download_file_to(&url, dir.path())
        .await
        .expect("download should succeed");

    let path = PathBuf::from(&result.path);
    assert!(path.exists());
    assert!(
        path.file_name().map(|n| n == "model.stl").unwrap_or(false),
        "filename should come from URL path, got {:?}",
        path.file_name()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "test content");
}
