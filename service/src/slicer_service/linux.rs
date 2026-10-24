use std::{path::PathBuf, process::Command};

use crate::service_error::ServiceError;

use super::Slicer;

impl Slicer {
    pub(super) fn find_installed(&self) -> Option<&'static str> {
        installed_flatpak_package(self)
    }

    pub(super) fn spawn_with_paths(
        package: &'static str,
        paths: Vec<PathBuf>,
    ) -> Result<(), ServiceError> {
        let _ = Command::new("flatpak")
            .arg("run")
            .arg("--file-forwarding")
            .arg(package)
            .arg("@@")
            .args(paths)
            .arg("@@")
            .spawn()?;

        Ok(())
    }
}

/// First installed Flatpak app-id for a slicer. `OrcaSlicer` migrated its Flathub package in
/// 2026 (`io.github.softfever.OrcaSlicer` -> `com.orcaslicer.OrcaSlicer`); listing both keeps
/// detection and launch working for pre- and post-migration installs.
fn installed_flatpak_package(slicer: &Slicer) -> Option<&'static str> {
    let packages: &[&str] = match slicer {
        Slicer::PrusaSlicer => &["com.prusa3d.PrusaSlicer"],
        Slicer::OrcaSlicer => &[
            "com.orcaslicer.OrcaSlicer",
            "io.github.softfever.OrcaSlicer",
        ],
        Slicer::Cura => &["com.ultimaker.cura"],
        Slicer::BambuStudio => &["com.bambulab.BambuStudio"],
        Slicer::Custom => &[],
    };

    packages.iter().copied().find(|package| {
        Command::new("flatpak")
            .arg("info")
            .arg(package)
            .output()
            .is_ok_and(|output| output.status.success())
    })
}
