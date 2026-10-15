use async_zip::error::ZipError;
use serde::{Serialize, Serializer};
use service::service_error;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ApplicationError {
    #[error("Failed to open or read file")]
    FileSystemFault(#[from] std::io::Error),
    #[error("Internal error")]
    InternalError(String),
    #[error("Failed to process JSON")]
    JsonError(#[from] serde_json::Error),
    #[error("Framework error")]
    FrameworkError(#[from] tauri::Error),
    #[error("Database error")]
    DatabaseError(#[from] db::DbError),
    #[error("Service error")]
    ServiceError(#[from] service::ServiceError),
    #[error("Web request error")]
    WebRequestError(#[from] tauri_plugin_http::reqwest::Error),
    #[error("Zip operation error")]
    AsyncZipOperationError(#[from] ZipError),
}

impl Serialize for ApplicationError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let (error_type, error_inner_message) = match self {
            Self::ServiceError(inner) => return inner.serialize(serializer),
            Self::FileSystemFault(inner) => ("FileSystemFault", inner.to_string()),
            Self::InternalError(s) => ("InternalError", s.clone()),
            Self::JsonError(inner) => ("JsonError", inner.to_string()),
            Self::FrameworkError(inner) => ("FrameworkError", inner.to_string()),
            Self::DatabaseError(inner) => ("DatabaseError", inner.to_string()),
            Self::WebRequestError(inner) => ("WebRequestError", inner.to_string()),
            Self::AsyncZipOperationError(inner) => ("AsyncZipOperationError", inner.to_string()),
        };

        service_error::serialize_error_struct(
            serializer,
            error_type,
            &self.to_string(),
            &error_inner_message,
        )
    }
}

#[cfg(test)]
mod tests {
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

    fn serialized(error: &super::ApplicationError) -> serde_json::Value {
        serde_json::to_value(error).expect("ApplicationError should serialize")
    }

    #[test]
    fn serializes_every_source_with_its_error_shape() {
        let json_error = serde_json::from_str::<()>("{").expect_err("truncated JSON should fail");
        let json_message = json_error.to_string();
        let framework_error = tauri::Error::AssetNotFound("index.html".to_string());
        let framework_message = framework_error.to_string();
        let web_request_error = tauri_plugin_http::reqwest::Client::new()
            .get("not a url")
            .build()
            .expect_err("a relative url should fail to build");
        let web_request_message = web_request_error.to_string();
        let cases = [
            (
                super::ApplicationError::from(std::io::Error::new(
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
                super::ApplicationError::InternalError("detail".to_string()),
                error_shape("InternalError", "Internal error", "detail"),
            ),
            (
                super::ApplicationError::from(json_error),
                error_shape("JsonError", "Failed to process JSON", &json_message),
            ),
            (
                super::ApplicationError::from(framework_error),
                error_shape("FrameworkError", "Framework error", &framework_message),
            ),
            (
                super::ApplicationError::from(db::DbError::RowNotFound),
                error_shape(
                    "DatabaseError",
                    "Database error",
                    &db::DbError::RowNotFound.to_string(),
                ),
            ),
            (
                super::ApplicationError::from(db::DbError::PoolTimedOut),
                error_shape(
                    "DatabaseError",
                    "Database error",
                    &db::DbError::PoolTimedOut.to_string(),
                ),
            ),
            (
                super::ApplicationError::from(web_request_error),
                error_shape("WebRequestError", "Web request error", &web_request_message),
            ),
            (
                super::ApplicationError::from(async_zip::error::ZipError::CDRsNotLoaded),
                error_shape(
                    "AsyncZipOperationError",
                    "Zip operation error",
                    &async_zip::error::ZipError::CDRsNotLoaded.to_string(),
                ),
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(serialized(&error), expected);
        }
    }

    #[tokio::test]
    async fn serializes_service_error_shape_through_unchanged() {
        let handle = tokio::spawn(std::future::pending::<()>());
        handle.abort();
        let join_error = handle
            .await
            .expect_err("an aborted task should not join successfully");
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
                service::ServiceError::from(async_zip::error::ZipError::CDRsNotLoaded),
                error_shape(
                    "ZipError",
                    "Failed to read or write zip file",
                    &async_zip::error::ZipError::CDRsNotLoaded.to_string(),
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

        for (service_error, expected) in cases {
            assert_eq!(
                serialized(&super::ApplicationError::from(service_error)),
                expected
            );
        }
    }
}
