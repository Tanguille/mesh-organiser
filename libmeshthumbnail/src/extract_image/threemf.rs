use std::path::Path;

use image::DynamicImage;

use crate::{error::MeshThumbnailError, parse_model::find_zip_entry_bytes, path_ext::matches_ext};

pub fn handle_threemf(input_path: &Path) -> Result<Option<DynamicImage>, MeshThumbnailError> {
    if matches_ext(input_path, "3mf") {
        Ok(Some(extract_image_from_3mf(input_path)?))
    } else {
        Ok(None)
    }
}

fn extract_image_from_3mf(input_path: &Path) -> Result<DynamicImage, MeshThumbnailError> {
    let buffer = find_zip_entry_bytes(
        input_path,
        |name| name.ends_with("thumbnail_middle.png"),
        "thumbnail_middle.png not found in 3mf file",
    )?;

    Ok(image::load_from_memory(&buffer)?)
}
