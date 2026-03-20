//! Resolve user-controlled path segments under a trusted base directory (path traversal mitigation).

use std::{
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;
use tokio::fs;

use crate::error::ApplicationError;

#[derive(Debug, Error)]
pub enum UnderBaseError {
    #[error("path escapes the base directory")]
    PathEscape,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Resolves `base.join(relative_path)` after canonicalizing both the base and the candidate path.
/// `relative_path` must be relative (not absolute); it is typically user-controlled.
/// Returns `Ok` only if the resolved path remains under `base` (after symlink and `..` resolution).
pub async fn canonical_path_under_base(
    base: &Path,
    relative_path: &str,
) -> Result<PathBuf, UnderBaseError> {
    if relative_path.is_empty() {
        return Err(UnderBaseError::PathEscape);
    }
    let path_under_base = Path::new(relative_path);
    if path_under_base.is_absolute() {
        return Err(UnderBaseError::PathEscape);
    }

    let canonical_base = fs::canonicalize(base).await?;
    let joined = canonical_base.join(path_under_base);
    let canonical = fs::canonicalize(&joined).await?;

    if !canonical.starts_with(&canonical_base) {
        return Err(UnderBaseError::PathEscape);
    }

    Ok(canonical)
}

/// Classified outcome of [`canonical_path_under_base`] for Axum routes (400 / 404 / 500).
#[derive(Debug)]
pub enum OpenUnderBaseError {
    BadRequest,
    NotFound,
    Io(io::Error),
}

impl From<UnderBaseError> for OpenUnderBaseError {
    fn from(value: UnderBaseError) -> Self {
        match value {
            UnderBaseError::PathEscape => Self::BadRequest,
            UnderBaseError::Io(e) if e.kind() == ErrorKind::NotFound => Self::NotFound,
            UnderBaseError::Io(e) => Self::Io(e),
        }
    }
}

impl OpenUnderBaseError {
    /// Maps to `Result<Response, ApplicationError>` so callers can `return err.respond()`.
    pub fn respond(self) -> Result<Response, ApplicationError> {
        match self {
            Self::BadRequest => Ok(StatusCode::BAD_REQUEST.into_response()),
            Self::NotFound => Ok(StatusCode::NOT_FOUND.into_response()),
            Self::Io(e) => Err(e.into()),
        }
    }
}

/// [`canonical_path_under_base`] with errors classified for HTTP handlers.
pub async fn resolve_path_under_base(
    base: &Path,
    relative_path: &str,
) -> Result<PathBuf, OpenUnderBaseError> {
    canonical_path_under_base(base, relative_path)
        .await
        .map_err(Into::into)
}

/// [`resolve_path_under_base`] with the process temp directory as `base`.
pub async fn resolve_path_under_temp(relative_path: &str) -> Result<PathBuf, OpenUnderBaseError> {
    resolve_path_under_base(&std::env::temp_dir(), relative_path).await
}

#[cfg(test)]
mod tests {
    use tokio::fs;

    use super::{OpenUnderBaseError, UnderBaseError, canonical_path_under_base};

    #[tokio::test]
    async fn resolves_subdirectory_under_base() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let inner = base.join("meshorganiser_ok");
        fs::create_dir(&inner).await.unwrap();

        let resolved = canonical_path_under_base(base, "meshorganiser_ok")
            .await
            .expect("expected path under base");

        assert!(resolved.ends_with("meshorganiser_ok"));
    }

    #[tokio::test]
    async fn rejects_when_canonical_path_leaves_base() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let inner = base.join("meshorganiser_inner");
        fs::create_dir(&inner).await.unwrap();

        let outside = base
            .parent()
            .expect("temp dir has parent")
            .join("meshorganiser_path_safety_outside");
        fs::create_dir(&outside).await.unwrap();

        let relative_path = "meshorganiser_inner/../../meshorganiser_path_safety_outside";
        let err = canonical_path_under_base(base, relative_path)
            .await
            .expect_err("expected path escape");

        assert!(matches!(err, UnderBaseError::PathEscape));
        let _ = fs::remove_dir(&outside).await;
    }

    #[tokio::test]
    async fn rejects_absolute_segment() {
        let dir = tempfile::tempdir().unwrap();
        let absolute_path = if cfg!(windows) {
            r"C:\Windows\System32"
        } else {
            "/etc"
        };
        let e = canonical_path_under_base(dir.path(), absolute_path)
            .await
            .expect_err("expected rejection");

        assert!(matches!(e, UnderBaseError::PathEscape));
    }

    #[test]
    fn open_under_base_error_maps_path_escape_to_bad_request() {
        let e = OpenUnderBaseError::from(UnderBaseError::PathEscape);
        assert!(matches!(e, OpenUnderBaseError::BadRequest));
    }
}
