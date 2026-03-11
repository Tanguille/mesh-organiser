use std::{
    fs::File,
    io::{BufRead, BufReader, Cursor, Read},
    path::Path,
};

use base64::{Engine, prelude::BASE64_STANDARD};
use image::{DynamicImage, ImageReader};
use zip::ZipArchive;

use crate::error::MeshThumbnailError;

pub fn handle_gcode(input_path: &Path) -> Result<Option<DynamicImage>, MeshThumbnailError> {
    let is_gcode = input_path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gcode"));
    let is_gcode_zip = input_path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        && input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.to_lowercase().ends_with(".gcode"));

    if is_gcode {
        Ok(Some(extract_image_from_gcode_file(input_path)?))
    } else if is_gcode_zip {
        Ok(Some(extract_image_from_gcode_zip(input_path)?))
    } else {
        Ok(None)
    }
}

struct GcodeImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl GcodeImage {
    const fn area(&self) -> u32 {
        self.width * self.height
    }
}

fn extract_image_from_gcode_file(gcode_path: &Path) -> Result<DynamicImage, MeshThumbnailError> {
    let mut file = File::open(gcode_path)?;
    extract_image_from_gcode(&mut file)
}

fn extract_image_from_gcode_zip(gcode_zip_path: &Path) -> Result<DynamicImage, MeshThumbnailError> {
    let file = File::open(gcode_zip_path)?;
    let mut zip = ZipArchive::new(file)?;

    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        if Path::new(file.name())
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gcode"))
        {
            return extract_image_from_gcode(&mut file);
        }
    }

    Err(MeshThumbnailError::InternalError(String::from(
        "No gcode file found in zip archive",
    )))
}

fn extract_image_from_gcode<W>(reader: &mut W) -> Result<DynamicImage, MeshThumbnailError>
where
    W: Read,
{
    let buffered_reader = BufReader::new(reader);
    let mut gcode_images: Vec<GcodeImage> = Vec::new();
    let mut in_gcode_section = false;
    let mut gcode_img_width = 0;
    let mut gcode_img_height = 0;
    let mut image = String::new();

    for line in buffered_reader.lines().map_while(Result::ok) {
        if line.starts_with("; thumbnail begin") {
            let Some(pixel_format) = line.split(' ').nth(3) else {
                continue;
            };

            let pixel_format_unpacked: Vec<u32> = pixel_format
                .split('x')
                .map(|f| f.parse().unwrap_or_default())
                .collect();

            gcode_img_width = *pixel_format_unpacked.first().unwrap_or(&0);
            gcode_img_height = *pixel_format_unpacked.get(1).unwrap_or(&0);
            image = String::new();

            in_gcode_section = gcode_img_width > 0 && gcode_img_height > 0;
        } else if line.starts_with("; thumbnail end") {
            in_gcode_section = false;
            let image = BASE64_STANDARD.decode(&image).map_err(|e| {
                MeshThumbnailError::InternalError(format!("Error decoding base64 image data: {e}"))
            })?;

            gcode_images.push(GcodeImage {
                width: gcode_img_width,
                height: gcode_img_height,
                data: image,
            });
        } else if in_gcode_section {
            image.push_str(line.get(2..).unwrap_or_default().trim());
        } else if line.starts_with("; EXECUTABLE_BLOCK_START") {
            break;
        }
    }

    gcode_images.sort_by_key(|b| std::cmp::Reverse(b.area()));

    let Some(largest_image) = gcode_images.first() else {
        return Err(MeshThumbnailError::InternalError(String::from(
            "No thumbnail found in gcode file",
        )));
    };

    let step1 = ImageReader::new(Cursor::new(&largest_image.data))
        .with_guessed_format()?
        .decode()?;

    Ok(step1)
}
