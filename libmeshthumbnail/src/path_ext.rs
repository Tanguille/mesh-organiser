use std::path::Path;

/// Returns `true` when `path`'s extension matches `ext` (case-insensitive).
#[must_use]
pub fn matches_ext(path: &Path, ext: &str) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// Returns `true` when `path` is a `.zip` whose inner stem ends with `.<ext>`
/// (case-insensitive), e.g. `model.stl.zip` for `ext = "stl"`.
#[must_use]
pub fn is_zip_of(path: &Path, ext: &str) -> bool {
    matches_ext(path, "zip")
        && path
            .file_stem()
            .is_some_and(|stem| matches_ext(Path::new(stem), ext))
}
