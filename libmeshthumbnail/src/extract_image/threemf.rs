use std::{io::Cursor, path::Path};

use image::{DynamicImage, ImageReader};

use crate::{error::MeshThumbnailError, parse_model::find_zip_entry_bytes, path_ext::matches_ext};

pub fn handle_threemf(input_path: &Path) -> Result<Option<DynamicImage>, MeshThumbnailError> {
    if matches_ext(input_path, "3mf") {
        Ok(Some(extract_image_from_3mf(input_path)?))
    } else {
        Ok(None)
    }
}

fn extract_image_from_3mf(input_path: &Path) -> Result<DynamicImage, MeshThumbnailError> {
    // Open 3mf path as zip file
    let buffer = find_zip_entry_bytes(
        input_path,
        |name| name.ends_with("thumbnail_middle.png"),
        "thumbnail_middle.png not found in 3mf file",
    )?;

    let step1 = ImageReader::new(Cursor::new(buffer))
        .with_guessed_format()?
        .decode()?;

    Ok(step1)
}
