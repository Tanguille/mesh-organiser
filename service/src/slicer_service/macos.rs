use std::path::PathBuf;

use crate::{app_state::AppState, service_error::ServiceError, slicer_service::open_custom_slicer};

use super::Slicer;

impl Slicer {
    pub fn is_installed(&self) -> bool {
        if let Slicer::Custom = self {
            return true;
        }

        get_slicer_path(self).is_some()
    }

    /// Like [`is_installed`](Self::is_installed), but runs detection on the blocking thread pool.
    pub async fn is_installed_async(&self) -> bool {
        let slicer = self.clone();

        tokio::task::spawn_blocking(move || slicer.is_installed())
            .await
            .unwrap_or(false)
    }

    pub async fn open(
        &self,
        paths: Vec<PathBuf>,
        app_state: &AppState,
    ) -> Result<(), ServiceError> {
        if let Slicer::Custom = self {
            return open_custom_slicer(paths, app_state).await;
        }

        if paths.is_empty() {
            return Err(ServiceError::InternalError(String::from(
                "No models to open",
            )));
        }

        println!("Opening in slicer: {:?}", paths);

        let slicer = self.clone();
        let maybe_path = tokio::task::spawn_blocking(move || get_slicer_path(&slicer))
            .await
            .map_err(|join_err| {
                ServiceError::InternalError(format!("Slicer path task failed: {join_err}"))
            })?;

        let Some(slicer_path) = maybe_path else {
            return Err(ServiceError::InternalError(String::from(
                "Slicer not installed",
            )));
        };

        let _child = tokio::process::Command::new("open")
            .arg("-a")
            .arg(slicer_path)
            .arg("--args")
            .args(paths)
            .spawn()?;

        Ok(())
    }
}

fn get_slicer_path(slicer: &Slicer) -> Option<PathBuf> {
    match slicer {
        Slicer::PrusaSlicer => {
            let path: PathBuf =
                PathBuf::from("/Applications/Original Prusa Drivers/PrusaSlicer.app");
            let second_path = PathBuf::from("/Applications/PrusaSlicer.app");

            if path.exists() {
                return Some(path);
            }

            if second_path.exists() {
                return Some(second_path);
            }

            return None;
        }
        Slicer::OrcaSlicer => {
            let path = PathBuf::from("/Applications/OrcaSlicer.app");
            if path.exists() {
                return Some(path);
            }
            return None;
        }
        Slicer::Cura => {
            let path = PathBuf::from("/Applications/UltiMaker Cura.app");
            if path.exists() {
                return Some(path);
            }
            return None;
        }
        Slicer::BambuStudio => {
            let path = PathBuf::from("/Applications/BambuStudio.app");
            if path.exists() {
                return Some(path);
            }
            return None;
        }
        _ => None,
    }
}
