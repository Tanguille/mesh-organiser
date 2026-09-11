//! Resolve user-controlled path segments under a trusted base directory (path traversal mitigation).

use std::{
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tokio::fs;

use crate::error::ApplicationError;

/// Classified outcome of [`resolve_path_under_base`] for Axum routes (400 / 404 / 500).
#[derive(Debug)]
pub enum OpenUnderBaseError {
    BadRequest,
    NotFound,
    Io(io::Error),
}

impl From<io::Error> for OpenUnderBaseError {
    fn from(value: io::Error) -> Self {
        if value.kind() == ErrorKind::NotFound {
            Self::NotFound
        } else {
            Self::Io(value)
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

/// Resolves `base.join(relative_path)` after canonicalizing both the base and the candidate path.
/// `relative_path` must be relative (not absolute); it is typically user-controlled.
/// Returns `Ok` only if the resolved path remains under `base` (after symlink and `..` resolution).
pub async fn resolve_path_under_base(
    base: &Path,
    relative_path: &str,
) -> Result<PathBuf, OpenUnderBaseError> {
    if relative_path.is_empty() {
        return Err(OpenUnderBaseError::BadRequest);
    }
    let path_under_base = Path::new(relative_path);
    if path_under_base.is_absolute() {
        return Err(OpenUnderBaseError::BadRequest);
    }

    let canonical_base = fs::canonicalize(base).await?;
    let joined = canonical_base.join(path_under_base);
    let canonical = fs::canonicalize(&joined).await?;

    if !canonical.starts_with(&canonical_base) {
        return Err(OpenUnderBaseError::BadRequest);
    }

    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use tokio::fs;

    use super::{OpenUnderBaseError, resolve_path_under_base};

    #[tokio::test]
    async fn resolves_subdirectory_under_base() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let inner = base.join("meshorganiser_ok");
        fs::create_dir(&inner).await.unwrap();

        let resolved = resolve_path_under_base(base, "meshorganiser_ok")
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
        let err = resolve_path_under_base(base, relative_path)
            .await
            .expect_err("expected path escape");

        assert!(matches!(err, OpenUnderBaseError::BadRequest));
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
        let e = resolve_path_under_base(dir.path(), absolute_path)
            .await
            .expect_err("expected rejection");

        assert!(matches!(e, OpenUnderBaseError::BadRequest));
    }
}
