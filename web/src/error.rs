use axum::{Json, extract::multipart::MultipartError, response::IntoResponse};
use serde::{Serialize, Serializer};
use thiserror::Error;
use tokio::task;

use service::{ServiceError, service_error};

/// Sources that `ServiceError` also wraps convert through it (see the `From` impls
/// below), so their serialised shape is defined once, in the service crate.
#[derive(Error, Debug)]
pub enum ApplicationError {
    #[error(transparent)]
    Service(#[from] ServiceError),
    #[error(transparent)]
    TaskJoinError(#[from] task::JoinError),
    #[error("Upload error")]
    MultipartError(#[from] MultipartError),
}

impl From<std::io::Error> for ApplicationError {
    fn from(e: std::io::Error) -> Self {
        Self::Service(e.into())
    }
}

impl From<db::DbError> for ApplicationError {
    fn from(e: db::DbError) -> Self {
        Self::Service(e.into())
    }
}

impl Serialize for ApplicationError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let (error_type, error_inner_message) = match self {
            Self::Service(inner) => return inner.serialize(serializer),
            Self::TaskJoinError(inner) => ("TaskJoinError", inner.to_string()),
            Self::MultipartError(inner) => ("MultipartError", inner.to_string()),
        };

        service_error::serialize_error_struct(
            serializer,
            error_type,
            &self.to_string(),
            &error_inner_message,
        )
    }
}

impl IntoResponse for ApplicationError {
    fn into_response(self) -> axum::response::Response {
        let json = serde_json::to_string(&self)
            .unwrap_or_else(|_| "Failed to serialize error".to_string());
        println!("[Error] {json}");
        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::ApplicationError;
    use axum::response::IntoResponse;

    #[test]
    fn internal_error_display() {
        let err = ApplicationError::from(service::ServiceError::InternalError(
            "something went wrong".to_string(),
        ));
        assert_eq!(err.to_string(), "Internal error");
    }

    #[test]
    fn internal_error_serializes_three_field_shape() {
        let err =
            ApplicationError::from(service::ServiceError::InternalError("detail".to_string()));
        let value = serde_json::to_value(&err).expect("serialization should succeed");
        let obj = value.as_object().expect("should be object");
        assert_eq!(
            obj.get("error_type").and_then(|v| v.as_str()),
            Some("InternalError")
        );
        assert_eq!(
            obj.get("error_message").and_then(|v| v.as_str()),
            Some("Internal error")
        );
        assert_eq!(
            obj.get("error_inner_message").and_then(|v| v.as_str()),
            Some("detail")
        );
    }

    #[test]
    fn into_response_returns_500() {
        let err = ApplicationError::from(service::ServiceError::InternalError("test".to_string()));
        let response = err.into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    fn error_shape(
        error_type: &str,
        error_message: &str,
        error_inner_message: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "error_type": error_type,
            "error_message": error_message,
            "error_inner_message": error_inner_message,
        })
    }

    /// Every `ApplicationError` reaches the client as a 500 with a JSON body, whatever its source.
    async fn internal_server_error_body(error: ApplicationError) -> serde_json::Value {
        let response = error.into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_TYPE],
            "application/json"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("error body should be readable");

        serde_json::from_slice(&body).expect("error body should be JSON")
    }

    async fn aborted_join_error() -> tokio::task::JoinError {
        let handle = tokio::spawn(std::future::pending::<()>());
        handle.abort();

        handle
            .await
            .expect_err("an aborted task should not join successfully")
    }

    async fn malformed_multipart_error() -> axum::extract::multipart::MultipartError {
        let request = axum::http::Request::builder()
            .header(
                axum::http::header::CONTENT_TYPE,
                "multipart/form-data; boundary=X",
            )
            .body(axum::body::Body::from("not a multipart body"))
            .expect("request should build");
        let mut multipart =
            <axum::extract::Multipart as axum::extract::FromRequest<()>>::from_request(
                request,
                &(),
            )
            .await
            .expect("a multipart content type should be accepted");

        multipart
            .next_field()
            .await
            .expect_err("a body without boundaries should fail to parse")
    }

    #[tokio::test]
    async fn into_response_maps_every_source_to_500_with_its_error_shape() {
        let join_error = aborted_join_error().await;
        let join_message = join_error.to_string();
        let multipart_error = malformed_multipart_error().await;
        let multipart_message = multipart_error.to_string();
        let cases = [
            (
                ApplicationError::from(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "missing.stl",
                )),
                error_shape(
                    "FileSystemFault",
                    "Failed to open or read file",
                    "missing.stl",
                ),
            ),
            (
                ApplicationError::from(service::ServiceError::InternalError("detail".to_string())),
                error_shape("InternalError", "Internal error", "detail"),
            ),
            (
                ApplicationError::from(db::DbError::RowNotFound),
                error_shape(
                    "DatabaseError",
                    "Database error",
                    &db::DbError::RowNotFound.to_string(),
                ),
            ),
            (
                ApplicationError::from(db::DbError::PoolTimedOut),
                error_shape(
                    "DatabaseError",
                    "Database error",
                    &db::DbError::PoolTimedOut.to_string(),
                ),
            ),
            (
                ApplicationError::from(join_error),
                error_shape("TaskJoinError", &join_message, &join_message),
            ),
            (
                ApplicationError::from(multipart_error),
                error_shape("MultipartError", "Upload error", &multipart_message),
            ),
        ];

        for (error, expected_body) in cases {
            assert_eq!(internal_server_error_body(error).await, expected_body);
        }
    }

    #[tokio::test]
    async fn into_response_passes_service_error_shape_through_unchanged() {
        let join_error = aborted_join_error().await;
        let join_message = join_error.to_string();
        let json_error = serde_json::from_str::<()>("{").expect_err("truncated JSON should fail");
        let json_message = json_error.to_string();
        let cases = [
            (
                service::ServiceError::InternalError("detail".to_string()),
                error_shape("InternalError", "Internal error", "detail"),
            ),
            (
                service::ServiceError::from(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "locked.3mf",
                )),
                error_shape(
                    "FileSystemFault",
                    "Failed to open or read file",
                    "locked.3mf",
                ),
            ),
            (
                service::ServiceError::from(db::DbError::RowNotFound),
                error_shape(
                    "DatabaseError",
                    "Database error",
                    &db::DbError::RowNotFound.to_string(),
                ),
            ),
            (
                service::ServiceError::from(json_error),
                error_shape("JsonError", "Failed to process JSON", &json_message),
            ),
            (
                service::ServiceError::from(join_error),
                error_shape(
                    "TaskExecutionFailedError",
                    "TaskExecutionFailedError",
                    &join_message,
                ),
            ),
        ];

        for (service_error, expected_body) in cases {
            assert_eq!(
                internal_server_error_body(ApplicationError::from(service_error)).await,
                expected_body
            );
        }
    }
}
