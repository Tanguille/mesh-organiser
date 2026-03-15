use axum::{Json, extract::multipart::MultipartError, response::IntoResponse};
use serde::{Serialize, Serializer};
use thiserror::Error;
use tokio::task;

use service::service_error;

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
            Self::DatabaseError(inner) => service_error::serialize_error_struct(
                serializer,
                "DatabaseError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::TaskJoinError(inner) => service_error::serialize_error_struct(
                serializer,
                "TaskJoinError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::MultipartError(inner) => service_error::serialize_error_struct(
                serializer,
                "MultipartError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::ServiceError(_) => unreachable!(),
        }
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
        let err = ApplicationError::InternalError("something went wrong".to_string());
        assert_eq!(err.to_string(), "Internal error");
    }

    #[test]
    fn internal_error_serializes_three_field_shape() {
        let err = ApplicationError::InternalError("detail".to_string());
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
        let err = ApplicationError::InternalError("test".to_string());
        let response = err.into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
