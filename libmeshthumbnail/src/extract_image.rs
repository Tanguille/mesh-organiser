use std::path::Path;

use image::DynamicImage;

mod gcode;
mod threemf;

/// Tries to extract a thumbnail image from the given path (e.g. 3MF, G-code). The first
/// handler that claims the path decides the result, including its error.
pub fn handle_extract_image(
    input_path: &Path,
) -> Result<Option<DynamicImage>, crate::error::MeshThumbnailError> {
    if let Some(image) = threemf::handle_threemf(input_path)? {
        return Ok(Some(image));
    }

    gcode::handle_gcode(input_path)
}
