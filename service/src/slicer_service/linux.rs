use std::{path::PathBuf, process::Command};

use crate::{app_state::AppState, service_error::ServiceError, slicer_service::open_custom_slicer};

use super::Slicer;

impl Slicer {
    #[must_use]
    pub fn is_installed(&self) -> bool {
        if matches!(self, Self::Custom) {
            return true;
        }

        let package = get_flatpak_slicer_package(self);

        if package.is_empty() {
            return false;
        }

        match Command::new("flatpak").arg("info").arg(package).output() {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    /// Like [`is_installed`](Self::is_installed), but runs detection on the blocking thread pool.
    pub async fn is_installed_async(&self) -> bool {
        let slicer = self.clone();

        tokio::task::spawn_blocking(move || slicer.is_installed())
            .await
            .unwrap_or(false)
    }

    /// Opens the slicer application with the given paths.
    ///
    /// # Errors
    ///
    /// Returns an error if the slicer is not installed or if no models are provided.
    pub async fn open(
        &self,
        paths: Vec<PathBuf>,
        app_state: &AppState,
    ) -> Result<(), ServiceError> {
        if matches!(self, Self::Custom) {
            return open_custom_slicer(paths, app_state).await;
        }

        if !self.is_installed_async().await {
            return Err(ServiceError::InternalError(String::from(
                "Slicer not installed",
            )));
        }

        println!("Opening in slicer: {paths:?}");

        if paths.is_empty() {
            return Err(ServiceError::InternalError(String::from(
                "No models to open",
            )));
        }

        let _child = tokio::process::Command::new("flatpak")
            .arg("run")
            .arg("--file-forwarding")
            .arg(get_flatpak_slicer_package(self))
            .arg("@@")
            .args(paths)
            .arg("@@")
            .spawn()?;

        Ok(())
    }
}

fn get_flatpak_slicer_package(slicer: &Slicer) -> String {
    match slicer {
        Slicer::PrusaSlicer => "com.prusa3d.PrusaSlicer",
        Slicer::OrcaSlicer => "io.github.softfever.OrcaSlicer",
        Slicer::Cura => "com.ultimaker.cura",
        Slicer::BambuStudio => "com.bambulab.BambuStudio",
        Slicer::Custom => "",
    }
    .to_string()
}
