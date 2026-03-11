use std::path::Path;

use image::DynamicImage;

mod gcode;
mod threemf;

/// Tries to extract a thumbnail image from the given path (e.g. 3MF, G-code).
///
/// # Errors
/// Returns an error when the file format is supported but reading or decoding fails.
pub fn handle_extract_image(
    input_path: &Path,
) -> Result<Option<DynamicImage>, crate::error::MeshThumbnailError> {
    if let Some(result) = threemf::handle_threemf(input_path)? {
        return Ok(Some(result));
    }

    if let Some(result) = gcode::handle_gcode(input_path)? {
        return Ok(Some(result));
    }

    Ok(None)
}
