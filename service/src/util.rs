use std::{
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

use regex::Regex;

use crate::service_error::ServiceError;

/// Returns a human-friendly file name (no path); strips extension if not a directory.
///
/// # Panics
///
/// Panics if the file name is not valid UTF-8.
#[must_use]
pub fn prettify_file_name(file: &Path, is_dir: bool) -> String {
    let extension = file.extension();
    let mut file_name: String = String::from(
        file.file_name()
            .unwrap_or_else(|| OsStr::new("unknown_filename"))
            .to_str()
            .unwrap(),
    );

    if !is_dir && let Some(ext) = extension {
        file_name = String::from(&file_name[0..file_name.len() - ext.len() - 1]);
    }

    let remove_whitespace = Regex::new(r" {2,}").unwrap();

    file_name = file_name.replace(['_', '-', '+'], " ");

    file_name = String::from(remove_whitespace.replace_all(&file_name, " "));

    file_name = String::from(file_name.trim());

    file_name
}

#[must_use]
pub fn cleanse_evil_from_name(name: &str) -> String {
    String::from(
        name.replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], " ")
            .trim(),
    )
}

/// Opens the given path in the system file explorer.
///
/// # Panics
///
/// Panics if the path is not valid UTF-8.
pub fn open_folder_in_explorer(path: &Path) {
    let path = path.to_str().unwrap();
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("explorer").arg(path).output().unwrap();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(path).output().unwrap();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("xdg-open").arg(path).output().unwrap();
    }
}

/// Returns the total size of all files in the directory (non-recursive).
///
/// # Panics
///
/// Panics if the directory cannot be read or a metadata call fails.
#[must_use]
pub fn get_folder_size(path: &PathBuf) -> u64 {
    std::fs::read_dir(path)
        .unwrap()
        .map(|f| f.unwrap().metadata().unwrap().len())
        .sum()
}

#[must_use]
pub fn is_zippable_file_extension(extension: &str) -> bool {
    let lowercase = extension.to_lowercase();

    ["stl", "obj", "step", "gcode"]
        .iter()
        .any(|f| lowercase.as_str().eq(*f))
}

#[must_use]
pub fn is_zipped_file_extension(extension: &str) -> bool {
    let lowercase = extension.to_lowercase();

    ["stl.zip", "obj.zip", "step.zip", "gcode.zip"]
        .iter()
        .any(|f| lowercase.as_str().eq(*f))
}

#[must_use]
pub fn convert_extension_to_zip(extension: &str) -> String {
    let lowercase = extension.to_lowercase();

    String::from(match lowercase.as_str() {
        "stl" => "stl.zip",
        "obj" => "obj.zip",
        "step" => "step.zip",
        "gcode" => "gcode.zip",
        _ => &lowercase,
    })
}

#[must_use]
pub fn convert_zip_to_extension(extension: &str) -> String {
    let lowercase = extension.to_lowercase();

    String::from(match lowercase.as_str() {
        "stl.zip" => "stl",
        "obj.zip" => "obj",
        "step.zip" => "step",
        "gcode.zip" => "gcode",
        _ => &lowercase,
    })
}

/// Reads the entire file as a UTF-8 string.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or read.
pub fn read_file_as_text(path: &Path) -> Result<String, ServiceError> {
    let mut file = std::fs::File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(contents)
}

// -----------------------------------------------------------------------------
// Regression tests for service::util pure helpers.
// We test these after consolidating util so that src-tauri uses service::util;
// the goal is to lock in behaviour and catch regressions from deduplication.
// Only pure functions are unit-tested here; IO (read_file_as_text,
// get_folder_size, open_folder_in_explorer) is left out unless tested via temp dir.
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        cleanse_evil_from_name, convert_extension_to_zip, convert_zip_to_extension,
        is_zippable_file_extension, is_zipped_file_extension, prettify_file_name,
    };

    // ---- cleanse_evil_from_name ----

    #[test]
    fn test_cleanse_evil_empty() {
        assert_eq!(cleanse_evil_from_name(""), "");
    }

    #[test]
    fn test_cleanse_evil_normal_unchanged() {
        assert_eq!(cleanse_evil_from_name("hello world"), "hello world");
    }

    #[test]
    fn test_cleanse_evil_removes_backslash() {
        assert_eq!(cleanse_evil_from_name("a\\b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_slash() {
        assert_eq!(cleanse_evil_from_name("a/b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_colon() {
        assert_eq!(cleanse_evil_from_name("a:b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_asterisk() {
        assert_eq!(cleanse_evil_from_name("a*b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_question_mark() {
        assert_eq!(cleanse_evil_from_name("a?b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_double_quote() {
        assert_eq!(cleanse_evil_from_name("a\"b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_less_than() {
        assert_eq!(cleanse_evil_from_name("a<b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_greater_than() {
        assert_eq!(cleanse_evil_from_name("a>b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_pipe() {
        assert_eq!(cleanse_evil_from_name("a|b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_trim_outer_whitespace() {
        assert_eq!(cleanse_evil_from_name("  x  "), "x");
    }

    // ---- prettify_file_name ----

    #[test]
    fn test_prettify_file_name_path_with_extension_is_file() {
        let path = PathBuf::from("some_dir/My_Cool-Model.stl");
        assert_eq!(prettify_file_name(&path, false), "My Cool Model");
    }

    #[test]
    fn test_prettify_file_name_path_with_extension_is_dir() {
        let path = PathBuf::from("some_dir/My_Folder.stl");
        assert_eq!(prettify_file_name(&path, true), "My Folder.stl");
    }

    #[test]
    fn test_prettify_file_name_no_extension() {
        let path = PathBuf::from("myfile");
        assert_eq!(prettify_file_name(&path, false), "myfile");
    }

    #[test]
    fn test_prettify_file_name_replaces_underscore_and_collapses_spaces() {
        let path = PathBuf::from("a__b___c.stl");
        assert_eq!(prettify_file_name(&path, false), "a b c");
    }

    // ---- is_zippable_file_extension ----

    #[test]
    fn test_is_zippable_stl() {
        assert!(is_zippable_file_extension("stl"));
    }

    #[test]
    fn test_is_zippable_stl_uppercase() {
        assert!(is_zippable_file_extension("STL"));
    }

    #[test]
    fn test_is_zippable_obj() {
        assert!(is_zippable_file_extension("obj"));
    }

    #[test]
    fn test_is_zippable_step() {
        assert!(is_zippable_file_extension("step"));
    }

    #[test]
    fn test_is_zippable_gcode() {
        assert!(is_zippable_file_extension("gcode"));
    }

    #[test]
    fn test_is_zippable_txt_returns_false() {
        assert!(!is_zippable_file_extension("txt"));
    }

    // ---- is_zipped_file_extension ----

    #[test]
    fn test_is_zipped_stl_zip() {
        assert!(is_zipped_file_extension("stl.zip"));
    }

    #[test]
    fn test_is_zipped_other_returns_false() {
        assert!(!is_zipped_file_extension("stl"));
        assert!(!is_zipped_file_extension("zip"));
        assert!(!is_zipped_file_extension("obj"));
    }

    // ---- convert_extension_to_zip ----

    #[test]
    fn test_convert_extension_to_zip_stl() {
        assert_eq!(convert_extension_to_zip("stl"), "stl.zip");
    }

    #[test]
    fn test_convert_extension_to_zip_obj_step_gcode() {
        assert_eq!(convert_extension_to_zip("obj"), "obj.zip");
        assert_eq!(convert_extension_to_zip("step"), "step.zip");
        assert_eq!(convert_extension_to_zip("gcode"), "gcode.zip");
    }

    #[test]
    fn test_convert_extension_to_zip_unknown_returns_lowercase() {
        assert_eq!(convert_extension_to_zip("TXT"), "txt");
        assert_eq!(convert_extension_to_zip("unknown"), "unknown");
    }

    // ---- convert_zip_to_extension ----

    #[test]
    fn test_convert_zip_to_extension_known() {
        assert_eq!(convert_zip_to_extension("stl.zip"), "stl");
        assert_eq!(convert_zip_to_extension("obj.zip"), "obj");
        assert_eq!(convert_zip_to_extension("step.zip"), "step");
        assert_eq!(convert_zip_to_extension("gcode.zip"), "gcode");
    }

    #[test]
    fn test_convert_zip_to_extension_unknown_returns_lowercase() {
        assert_eq!(convert_zip_to_extension("other.zip"), "other.zip");
        assert_eq!(convert_zip_to_extension("STL.ZIP"), "stl");
    }
}
