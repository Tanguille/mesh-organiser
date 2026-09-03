use std::io;

use async_zip::error::ZipError;
use serde::{Serialize, Serializer};
use service::service_error;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ApplicationError {
    #[error("Failed to open or read file")]
    FileSystemFault(#[from] io::Error),
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
