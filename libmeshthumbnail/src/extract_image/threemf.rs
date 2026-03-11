use std::{
    fs::File,
    io::{self, Cursor},
    path::Path,
};

use image::{DynamicImage, ImageReader};
use zip::ZipArchive;

use crate::error::MeshThumbnailError;

pub fn handle_threemf(input_path: &Path) -> Result<Option<DynamicImage>, MeshThumbnailError> {
    if input_path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"))
    {
        Ok(Some(extract_image_from_3mf(input_path)?))
    } else {
        Ok(None)
    }
}

fn extract_image_from_3mf(input_path: &Path) -> Result<DynamicImage, MeshThumbnailError> {
    // Open 3mf path as zip file
    let file = File::open(input_path)?;
    let mut zip = ZipArchive::new(file)?;

    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        if file.name().ends_with("thumbnail_middle.png") {
            let mut buffer = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0));
            io::copy(&mut file, &mut buffer)?;

            let step1 = ImageReader::new(Cursor::new(buffer))
                .with_guessed_format()?
                .decode()?;

            return Ok(step1);
        }
    }

    Err(MeshThumbnailError::InternalError(String::from(
        "thumbnail_middle.png not found in 3mf file",
    )))
}
