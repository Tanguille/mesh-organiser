use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use winreg::{HKEY, RegKey, enums};

use crate::{app_state::AppState, service_error::ServiceError, slicer_service::open_with_paths};

use super::{Slicer, open_custom_slicer};

impl Slicer {
    #[must_use]
    pub fn is_installed(&self) -> bool {
        if matches!(self, Self::Custom) {
            return true;
        }

        get_slicer_path(self).is_some()
    }

    /// Opens the slicer with the given model paths.
    ///
    /// # Errors
    ///
    /// Returns an error if the slicer is not installed or spawning the process fails.
    ///
    /// # Panics
    ///
    /// Panics if the slicer path cannot be converted to UTF-8.
    pub async fn open(
        &self,
        paths: Vec<PathBuf>,
        app_state: &AppState,
    ) -> Result<(), ServiceError> {
        if matches!(self, Self::Custom) {
            return open_custom_slicer(paths, app_state).await;
        }

        if !self.is_installed() {
            return Err(ServiceError::InternalError(String::from(
                "Slicer not installed",
            )));
        }

        let slicer_pathbuf = get_slicer_path(self).unwrap();
        let slicer_path = slicer_pathbuf.to_str().unwrap();

        println!("Opening in slicer: {paths:?}");

        open_with_paths(slicer_path, paths)
    }
}

fn get_registry_key(root: HKEY, subkey: &str, field: &str) -> Option<String> {
    let reg_key_result = RegKey::predef(root).open_subkey(subkey);

    if reg_key_result.is_err() {
        return None;
    }

    let reg_key = reg_key_result.unwrap();

    let value: Result<OsString, std::io::Error> = reg_key.get_value(field);

    value.map_or(None, |s| Some(s.to_str().unwrap().to_string()))
}

fn get_slicer_path(slicer: &Slicer) -> Option<PathBuf> {
    match slicer {
        Slicer::PrusaSlicer => {
            let key = get_registry_key(
                enums::HKEY_LOCAL_MACHINE,
                "SOFTWARE\\Prusa3D\\PrusaSlicer\\Settings",
                "InstallPath",
            );

            if let Some(key) = key {
                let path = PathBuf::from(key);

                if path.exists() {
                    return Some(path);
                }
            }

            let path = PathBuf::from("C:\\Program Files\\Prusa3D\\PrusaSlicer\\prusa-slicer.exe");

            if path.exists() {
                return Some(path);
            }

            None
        }
        Slicer::BambuStudio => {
            if let Some(key) = get_registry_key(
                enums::HKEY_LOCAL_MACHINE,
                "SOFTWARE\\Bambulab\\Bambu Studio",
                "InstallPath",
            ) {
                let path = PathBuf::from(key).join("bambu-studio.exe");

                if path.exists() {
                    return Some(path);
                }
            }

            let path = PathBuf::from("C:\\Program Files\\Bambu Studio\\bambu-studio.exe");

            if path.exists() {
                return Some(path);
            }

            None
        }
        Slicer::OrcaSlicer => {
            if let Some(key) = get_registry_key(
                enums::HKEY_LOCAL_MACHINE,
                "SOFTWARE\\WOW6432Node\\SoftFever\\OrcaSlicer",
                "",
            ) {
                let path = PathBuf::from(key).join("orca-slicer.exe");

                if path.exists() {
                    return Some(path);
                }
            }

            let path = PathBuf::from("C:\\Program Files\\OrcaSlicer\\orca-slicer.exe");

            if path.exists() {
                return Some(path);
            }

            None
        }
        Slicer::Cura => {
            let program_files = "C:\\Program Files";
            if let Ok(entries) = fs::read_dir(program_files) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                        && let Some(folder_name) = entry.file_name().to_str()
                        && folder_name.starts_with("UltiMaker Cura")
                    {
                        let exe_path = Path::new(program_files)
                            .join(folder_name)
                            .join("UltiMaker-Cura.exe");
                        if exe_path.exists() {
                            return Some(exe_path);
                        }
                    }
                }
            }

            None
        }
        Slicer::Custom => None,
    }
}
