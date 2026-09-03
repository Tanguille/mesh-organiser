use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use winreg::{HKEY, RegKey, enums};

use crate::{service_error::ServiceError, slicer_service::open_with_args_and_paths};

use super::Slicer;

impl Slicer {
    pub(super) fn detect_installed(&self) -> bool {
        get_slicer_path(self).is_some()
    }

    /// # Panics
    ///
    /// Panics if the slicer path cannot be resolved or converted to UTF-8.
    pub(super) fn spawn_with_paths(&self, paths: Vec<PathBuf>) -> Result<(), ServiceError> {
        let slicer_pathbuf = get_slicer_path(self).unwrap();

        open_with_args_and_paths(slicer_pathbuf.to_str().unwrap(), &[], paths)
    }
}

fn get_registry_key(root: HKEY, subkey: &str, field: &str) -> Option<String> {
    let reg_key_result = RegKey::predef(root).open_subkey(subkey);

    if reg_key_result.is_err() {
        return None;
    }

    let reg_key = reg_key_result.unwrap();

    let value: Result<OsString, io::Error> = reg_key.get_value(field);

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
                    if entry.file_type().is_ok_and(|ft| ft.is_dir())
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
