use std::{path::PathBuf, process::Command};

use crate::service_error::ServiceError;

use super::Slicer;

impl Slicer {
    pub(super) fn detect_installed(&self) -> bool {
        let package = get_flatpak_slicer_package(self);

        if package.is_empty() {
            return false;
        }

        match Command::new("flatpak").arg("info").arg(package).output() {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    pub(super) fn spawn_with_paths(&self, paths: Vec<PathBuf>) -> Result<(), ServiceError> {
        let _ = Command::new("flatpak")
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
