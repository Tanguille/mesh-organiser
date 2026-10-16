use std::{path::PathBuf, process::Command};

use crate::service_error::ServiceError;

use super::Slicer;

impl Slicer {
    pub(super) fn detect_installed(&self) -> bool {
        flatpak_packages_for(self)
            .iter()
            .any(|package| flatpak_installed(package))
    }

    pub(super) fn spawn_with_paths(&self, paths: Vec<PathBuf>) -> Result<(), ServiceError> {
        // Launch with the installed app-id; fall back to the preferred one when
        // nothing is installed (preserves the previous single-id behaviour).
        let preferred = flatpak_packages_for(self)[0];
        let package = flatpak_packages_for(self)
            .iter()
            .copied()
            .find(|package| flatpak_installed(package))
            .unwrap_or(preferred);

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

/// Flatpak app-id(s) for a slicer. OrcaSlicer migrated its Flathub package in 2026
/// (io.github.softfever.OrcaSlicer -> com.orcaslicer.OrcaSlicer); listing both keeps
/// detection and launch working for pre- and post-migration installs.
fn flatpak_packages_for(slicer: &Slicer) -> &'static [&'static str] {
    match slicer {
        Slicer::PrusaSlicer => &["com.prusa3d.PrusaSlicer"],
        Slicer::OrcaSlicer => &[
            "com.orcaslicer.OrcaSlicer",
            "io.github.softfever.OrcaSlicer",
        ],
        Slicer::Cura => &["com.ultimaker.cura"],
        Slicer::BambuStudio => &["com.bambulab.BambuStudio"],
        Slicer::Custom => &[""],
    }
}

fn flatpak_installed(package: &str) -> bool {
    !package.is_empty()
        && Command::new("flatpak")
            .arg("info")
            .arg(package)
            .output()
            .is_ok_and(|output| output.status.success())
}
