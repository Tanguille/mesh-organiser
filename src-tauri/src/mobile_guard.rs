//! Reject local-desktop-only Tauri commands on Android/iOS before they touch SQLite, blobs, or the filesystem.

use crate::error::ApplicationError;

/// Fails on mobile targets; no-op on desktop (compile-time `cfg!`, inlined).
#[inline]
pub fn require_local_desktop_app() -> Result<(), ApplicationError> {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return Err(ApplicationError::InternalError(
            "This action requires the local desktop app and is not available on mobile.".into(),
        ));
    }

    Ok(())
}
