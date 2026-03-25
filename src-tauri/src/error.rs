use async_zip::error::ZipError;
use serde::{Serialize, Serializer};
use thiserror::Error;

use service::service_error;

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
        if let Self::ServiceError(inner) = self {
            return inner.serialize(serializer);
        }

        match self {
            Self::FileSystemFault(inner) => service_error::serialize_error_struct(
                serializer,
                "FileSystemFault",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::InternalError(s) => service_error::serialize_error_struct(
                serializer,
                "InternalError",
                &self.to_string(),
                s,
            ),
            Self::JsonError(inner) => service_error::serialize_error_struct(
                serializer,
                "JsonError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::FrameworkError(inner) => service_error::serialize_error_struct(
                serializer,
                "FrameworkError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::DatabaseError(inner) => service_error::serialize_error_struct(
                serializer,
                "DatabaseError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::WebRequestError(inner) => service_error::serialize_error_struct(
                serializer,
                "WebRequestError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::AsyncZipOperationError(inner) => service_error::serialize_error_struct(
                serializer,
                "AsyncZipOperationError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::ServiceError(_) => unreachable!(),
        }
    }
}
