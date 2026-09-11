use std::{path::PathBuf, process::Command};

use crate::service_error::ServiceError;

use super::Slicer;

impl Slicer {
    pub(super) fn detect_installed(&self) -> bool {
        get_slicer_path(self).is_some()
    }

    /// # Panics
    ///
    /// Panics if the slicer path cannot be resolved.
    pub(super) fn spawn_with_paths(&self, paths: Vec<PathBuf>) -> Result<(), ServiceError> {
        let slicer_path = get_slicer_path(self).unwrap();

        Command::new("open")
            .arg("-a")
            .arg(slicer_path)
            .arg("--args")
            .args(paths)
            .spawn()?;

        Ok(())
    }
}

fn get_slicer_path(slicer: &Slicer) -> Option<PathBuf> {
    let candidates: &[&str] = match slicer {
        Slicer::PrusaSlicer => &[
            "/Applications/Original Prusa Drivers/PrusaSlicer.app",
            "/Applications/PrusaSlicer.app",
        ],
        Slicer::OrcaSlicer => &["/Applications/OrcaSlicer.app"],
        Slicer::Cura => &["/Applications/UltiMaker Cura.app"],
        Slicer::BambuStudio => &["/Applications/BambuStudio.app"],
        Slicer::Custom => &[],
    };

    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}
