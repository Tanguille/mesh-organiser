use std::path::Path;

mod gcode;
mod obj;
#[cfg(all(feature = "step", any(target_os = "linux", target_os = "windows")))]
mod step;
mod stl;
mod threemf;
#[cfg(all(feature = "step", any(target_os = "linux", target_os = "windows")))]
pub use step::convert_step_path_to_stl;
#[cfg(all(feature = "step", any(target_os = "linux", target_os = "windows")))]
pub use step::convert_step_to_stl;

/// Parses a mesh from the given path (STL, OBJ, 3MF, G-code, etc.).
///
/// # Errors
/// Returns an error when the format is supported but reading or parsing fails.
pub fn handle_parse(
    path: &Path,
) -> Result<Option<crate::mesh::Mesh>, crate::error::MeshThumbnailError> {
    if let Some(mesh) = stl::handle_stl(path)? {
        return Ok(Some(mesh));
    }

    if let Some(mesh) = obj::handle(path)? {
        return Ok(Some(mesh));
    }

    if let Some(mesh) = threemf::handle_threemf(path)? {
        return Ok(Some(mesh));
    }

    if let Some(mesh) = gcode::handle_gcode(path)? {
        return Ok(Some(mesh));
    }

    #[cfg(all(feature = "step", any(target_os = "linux", target_os = "windows")))]
    if let Some(mesh) = step::handle_step(path)? {
        return Ok(Some(mesh));
    }

    Ok(None)
}
