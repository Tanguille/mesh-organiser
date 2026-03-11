use axum::{Json, extract::multipart::MultipartError, response::IntoResponse};
use serde::{Serialize, Serializer};
use service::service_error;
use thiserror::Error;
use tokio::task;

#[derive(Error, Debug)]
pub enum ApplicationError {
    #[error("Failed to open or read file")]
    FileSystemFault(#[from] std::io::Error),
    #[error("Internal error")]
    InternalError(String),
    #[error("Failed to process JSON")]
    JsonError(#[from] serde_json::Error),
    #[error("Database error")]
    DatabaseError(#[from] db::DbError),
    #[error("Service error")]
    ServiceError(#[from] service::ServiceError),
    #[error(transparent)]
    TaskJoinError(#[from] task::JoinError),
    #[error("Upload error")]
    MultipartError(#[from] MultipartError),
}

impl Serialize for ApplicationError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let ApplicationError::ServiceError(inner) = self {
            return inner.serialize(serializer);
        }

        match self {
            ApplicationError::FileSystemFault(inner) => service_error::serialize_error_struct(
                serializer,
                "FileSystemFault",
                &self.to_string(),
                &inner.to_string(),
            ),
            ApplicationError::InternalError(s) => service_error::serialize_error_struct(
                serializer,
                "InternalError",
                &self.to_string(),
                s,
            ),
            ApplicationError::JsonError(inner) => service_error::serialize_error_struct(
                serializer,
                "JsonError",
                &self.to_string(),
                &inner.to_string(),
            ),
            ApplicationError::DatabaseError(inner) => service_error::serialize_error_struct(
                serializer,
                "DatabaseError",
                &self.to_string(),
                &inner.to_string(),
            ),
            ApplicationError::TaskJoinError(inner) => service_error::serialize_error_struct(
                serializer,
                "TaskJoinError",
                &self.to_string(),
                &inner.to_string(),
            ),
            ApplicationError::MultipartError(inner) => service_error::serialize_error_struct(
                serializer,
                "MultipartError",
                &self.to_string(),
                &inner.to_string(),
            ),
            ApplicationError::ServiceError(_) => unreachable!(),
        }
    }
}

impl IntoResponse for ApplicationError {
    fn into_response(self) -> axum::response::Response {
        let json = serde_json::to_string(&self).unwrap_or("Failed to serialize error".to_string());
        println!("[Error] {}", json);
        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
    }
}
