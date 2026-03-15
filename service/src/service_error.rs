use serde::{Serialize, Serializer, ser::SerializeStruct};
use thiserror::Error;

/// Serialises the common 3-field error shape so that service, tauri and web can share one implementation.
///
/// # Errors
///
/// Returns `S::Error` if serialisation fails.
pub fn serialize_error_struct<S>(
    serializer: S,
    error_type: &str,
    error_message: &str,
    error_inner_message: &str,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut state = serializer.serialize_struct("ServiceError", 3)?;
    state.serialize_field("error_type", error_type)?;
    state.serialize_field("error_message", error_message)?;
    state.serialize_field("error_inner_message", error_inner_message)?;
    state.end()
}

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("Failed to open or read file")]
    FileSystemFault(#[from] std::io::Error),
    #[error("Failed to read or write zip file")]
    ZipError(#[from] async_zip::error::ZipError),
    #[error("Internal error")]
    InternalError(String),
    #[error("Failed to download file")]
    DownloadError(#[from] reqwest::Error),
    #[error("Failed to process JSON")]
    JsonError(#[from] serde_json::Error),
    #[error("Database error")]
    DatabaseError(#[from] db::DbError),
    #[error("TaskExecutionFailedError")]
    TaskExecutionFailedError(#[from] tokio::task::JoinError),
    #[error("Threemf parsing error")]
    ThreemfError(#[from] threemf::Error),
    #[error("Thumbnail generation error: {0}")]
    ThumbnailError(#[from] image::ImageError),
}

impl Serialize for ServiceError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::FileSystemFault(inner) => serialize_error_struct(
                serializer,
                "FileSystemFault",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::ZipError(inner) => serialize_error_struct(
                serializer,
                "ZipError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::InternalError(s) => {
                serialize_error_struct(serializer, "InternalError", &self.to_string(), s)
            }
            Self::DownloadError(inner) => serialize_error_struct(
                serializer,
                "DownloadError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::JsonError(inner) => serialize_error_struct(
                serializer,
                "JsonError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::DatabaseError(inner) => serialize_error_struct(
                serializer,
                "DatabaseError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::TaskExecutionFailedError(inner) => serialize_error_struct(
                serializer,
                "TaskExecutionFailedError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::ThreemfError(inner) => serialize_error_struct(
                serializer,
                "ThreemfError",
                &self.to_string(),
                &inner.to_string(),
            ),
            Self::ThumbnailError(inner) => serialize_error_struct(
                serializer,
                "ThumbnailError",
                &self.to_string(),
                &inner.to_string(),
            ),
        }
    }
}

// -----------------------------------------------------------------------------
// Regression tests: we test the serialised error shape shared by service,
// tauri and web so that the 3-field JSON (error_type, error_message,
// error_inner_message) remains stable and frontend/tauri don't break.
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use serde::{Serialize, Serializer};

    use super::*;

    /// Wrapper that serialises via `serialize_error_struct` so we can test the helper directly.
    #[allow(clippy::struct_field_names)] // Names must match serialised JSON keys.
    #[derive(Debug)]
    struct TestErrorShape<'a> {
        error_type: &'a str,
        error_message: &'a str,
        error_inner_message: &'a str,
    }

    impl Serialize for TestErrorShape<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serialize_error_struct(
                serializer,
                self.error_type,
                self.error_message,
                self.error_inner_message,
            )
        }
    }

    #[test]
    fn serialize_error_struct_produces_object_with_exactly_three_string_fields() {
        // Arrange
        let shape = TestErrorShape {
            error_type: "TestType",
            error_message: "Something failed",
            error_inner_message: "inner detail",
        };

        // Act
        let value = serde_json::to_value(&shape).expect("serialize_error_struct should not fail");

        // Assert: result is an object with exactly the three keys and string values
        assert!(value.is_object(), "expected a JSON object");
        let obj = value.as_object().unwrap();
        assert_eq!(obj.len(), 3, "expected exactly 3 keys");
        assert_eq!(
            obj.get("error_type").and_then(|v| v.as_str()),
            Some("TestType")
        );
        assert_eq!(
            obj.get("error_message").and_then(|v| v.as_str()),
            Some("Something failed")
        );
        assert_eq!(
            obj.get("error_inner_message").and_then(|v| v.as_str()),
            Some("inner detail")
        );
    }

    #[test]
    fn service_error_internal_error_serialises_with_three_field_shape() {
        // Arrange
        let err = ServiceError::InternalError("custom detail".to_string());

        // Act
        let value = serde_json::to_value(&err).expect("ServiceError serialisation should not fail");

        // Assert: JSON has error_type, error_message, error_inner_message
        assert!(value.is_object());
        let obj = value.as_object().unwrap();
        assert_eq!(obj.len(), 3);
        assert_eq!(
            obj.get("error_type").and_then(|v| v.as_str()),
            Some("InternalError")
        );
        let error_message = obj.get("error_message").and_then(|v| v.as_str()).unwrap();
        let error_inner = obj
            .get("error_inner_message")
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(error_inner, "custom detail");
        // Display message is included in error_message
        assert!(
            error_message.contains("Internal error"),
            "error_message should contain Display text, got: {error_message}"
        );
    }

    #[test]
    fn service_error_json_error_serialises_with_three_field_shape() {
        // Arrange: produce a real serde_json::Error
        let json_err = serde_json::from_str::<()>("{").unwrap_err();
        let err = ServiceError::JsonError(json_err);

        // Act
        let value = serde_json::to_value(&err).expect("ServiceError serialisation should not fail");

        // Assert: same 3-field shape
        assert!(value.is_object());
        let obj = value.as_object().unwrap();
        assert_eq!(obj.len(), 3);
        assert_eq!(
            obj.get("error_type").and_then(|v| v.as_str()),
            Some("JsonError")
        );
        let error_message = obj.get("error_message").and_then(|v| v.as_str()).unwrap();
        assert!(
            error_message.contains("JSON") || error_message.contains("json"),
            "error_message should reflect JSON failure, got: {error_message}"
        );
        assert!(
            obj.get("error_inner_message")
                .and_then(|v| v.as_str())
                .is_some()
        );
    }
}
