use std::{
    panic,
    path::{Path, PathBuf},
};

use image::{DynamicImage, imageops::FilterType::Triangle};
use libmeshthumbnail::{extract_image, parse_model, render};
use tokio::task::JoinSet;
use vek::{Vec2, Vec3};

use db::{blob_db, model::blob::Blob, model::blob::FileType};

use crate::{
    AppState, ServiceError,
    export_service::{get_image_path_for_blob, get_model_path_for_blob},
    import_state::{ImportState, ImportStatus},
};

const IMAGE_WIDTH: usize = 400;
const IMAGE_HEIGHT: usize = 400;
const IMAGE_SIZE: Vec2<usize> = Vec2::new(IMAGE_WIDTH, IMAGE_HEIGHT);

fn resize_and_save_thumbnail(
    image: &mut DynamicImage,
    image_path: &Path,
) -> Result<(), ServiceError> {
    *image = image.resize_to_fill(
        u32::try_from(IMAGE_WIDTH).unwrap_or(400),
        u32::try_from(IMAGE_HEIGHT).unwrap_or(400),
        Triangle,
    );
    image.save(image_path)?;
    Ok(())
}

fn render(
    model_path: &Path,
    image_path: &Path,
    color: Vec3<u8>,
    rotation: Vec3<f32>,
) -> Result<(), ServiceError> {
    let mesh = match parse_model::handle_parse(model_path) {
        Ok(Some(mesh)) => mesh,
        Ok(None) => {
            return Err(ServiceError::InternalError(format!(
                "Could not generate thumbnail for model at path {}: unsupported format",
                model_path.display()
            )));
        }
        Err(e) => {
            return Err(ServiceError::InternalError(format!(
                "Error parsing model at path {} for thumbnail generation: {e}",
                model_path.display()
            )));
        }
    };

    let thumbnail = render::render(&mesh, IMAGE_SIZE, rotation, color, 1.0);

    DynamicImage::ImageRgba8(thumbnail).save(image_path)?;

    Ok(())
}

/// Returns the file type for the path if thumbnail generation is supported, or an error if unsupported.
/// Used by `process` and by tests (via `.to_extension()`) to lock in extension behaviour.
fn thumbnail_extension_for_path(path: &Path) -> Result<FileType, ServiceError> {
    let file_type = FileType::from_pathbuf(path);
    if file_type.is_unsupported() {
        return Err(ServiceError::InternalError(format!(
            "Unsupported file extension for thumbnail generation: {}",
            path.display()
        )));
    }
    // Reject Step (thumbnail render does not support it); allow Stl, Obj, Gcode, Threemf and their zips
    if file_type.is_step() {
        return Err(ServiceError::InternalError(format!(
            "Unsupported file extension for thumbnail generation: {}",
            path.display()
        )));
    }
    Ok(file_type)
}

fn process(
    model_path: &Path,
    image_path: &Path,
    color: Vec3<u8>,
    rotation: Vec3<f32>,
    fallback_3mf_thumbnail: bool,
    prefer_3mf_thumbnail: bool,
    prefer_gcode_thumbnail: bool,
) -> Result<(), ServiceError> {
    let file_type = thumbnail_extension_for_path(model_path)?;

    if ((prefer_3mf_thumbnail && file_type.is_3mf())
        || (prefer_gcode_thumbnail && file_type.is_gcode()))
        && let Ok(Some(mut image)) = extract_image::handle_extract_image(model_path)
    {
        resize_and_save_thumbnail(&mut image, image_path)?;
        return Ok(());
    }

    if render(model_path, image_path, color, rotation).is_ok() {
        return Ok(());
    }

    if fallback_3mf_thumbnail
        && !prefer_3mf_thumbnail
        && file_type.is_3mf()
        && let Ok(Some(mut image)) = extract_image::handle_extract_image(model_path)
    {
        resize_and_save_thumbnail(&mut image, image_path)?;
        return Ok(());
    }

    Err(ServiceError::InternalError(format!(
        "Failed to generate thumbnail for model at path {}",
        model_path.display()
    )))
}

/// Generates thumbnails for all blobs in the database.
///
/// # Errors
///
/// Returns an error if blob listing or thumbnail generation fails.
pub async fn generate_all_thumbnails(
    app_state: &AppState,
    overwrite: bool,
    import_state: &mut ImportState,
) -> Result<(), ServiceError> {
    let blobs = blob_db::get_blobs(&app_state.db).await?;
    let blob_refs: Vec<&Blob> = blobs.iter().collect();

    generate_thumbnails(&blob_refs, app_state, overwrite, import_state).await
}

/// Generates thumbnails for the given blobs.
///
/// # Errors
///
/// Returns an error if thumbnail generation fails.
///
/// # Panics
///
/// Panics if a spawned thumbnail task panics.
pub async fn generate_thumbnails(
    models: &[&Blob],
    app_state: &AppState,
    overwrite: bool,
    import_state: &mut ImportState,
) -> Result<(), ServiceError> {
    import_state.update_status(ImportStatus::ProcessingThumbnails);
    let fallback_3mf_thumbnail = app_state.get_configuration().fallback_3mf_thumbnail;
    let prefer_3mf_thumbnail = app_state.get_configuration().prefer_3mf_thumbnail;
    let prefer_gcode_thumbnail = app_state.get_configuration().prefer_gcode_thumbnail;
    let max_concurrent = app_state.get_configuration().core_parallelism;
    let rotation_setting = app_state.get_configuration().thumbnail_rotation;
    let rotation = Vec3::new(
        f32::from(rotation_setting[0]),
        f32::from(rotation_setting[1]),
        f32::from(rotation_setting[2]),
    );

    let color = app_state
        .get_configuration()
        .thumbnail_color
        .replace('#', "")
        .to_uppercase();

    let color = u32::from_str_radix(&color, 16).unwrap_or(0xEE_EE_EE);

    let color = Vec3::new(
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    );

    let paths: Vec<(PathBuf, PathBuf)> = models
        .iter()
        .map(|blob| {
            let model_path = get_model_path_for_blob(blob, app_state);
            let image_path = get_image_path_for_blob(blob, app_state);

            (model_path, image_path)
        })
        .filter(|(_, image_path)| overwrite || !image_path.exists())
        .collect();

    import_state.update_total_model_count(paths.len());

    let mut futures = JoinSet::new();
    let mut active = 0;

    for (model_path, image_path) in paths {
        futures.spawn_blocking(move || {
            // Ignore errors for now
            let _ = process(
                &model_path,
                &image_path,
                color,
                rotation,
                fallback_3mf_thumbnail,
                prefer_3mf_thumbnail,
                prefer_gcode_thumbnail,
            );
        });
        active += 1;

        if active >= max_concurrent
            && let Some(res) = futures.join_next().await
        {
            match res {
                Err(err) if err.is_panic() => panic::resume_unwind(err.into_panic()),
                Err(err) => panic!("{err}"),
                Ok(()) => {
                    active -= 1;
                    import_state.update_finished_thumbnails_count(1);
                }
            }
        }
    }

    futures.join_all().await;
    import_state.update_finished_thumbnails_count(
        import_state.model_count - import_state.finished_thumbnails_count,
    );
    import_state.update_status(ImportStatus::FinishedThumbnails);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::ServiceError;

    use super::thumbnail_extension_for_path;

    #[test]
    fn thumbnail_extension_stl() {
        assert_eq!(
            thumbnail_extension_for_path(&PathBuf::from("x.stl"))
                .unwrap()
                .to_extension(),
            "stl"
        );
    }

    #[test]
    fn thumbnail_extension_stl_uppercase() {
        assert_eq!(
            thumbnail_extension_for_path(&PathBuf::from("x.STL"))
                .unwrap()
                .to_extension(),
            "stl"
        );
    }

    #[test]
    fn thumbnail_extension_obj() {
        assert_eq!(
            thumbnail_extension_for_path(&PathBuf::from("model.obj"))
                .unwrap()
                .to_extension(),
            "obj"
        );
    }

    #[test]
    fn thumbnail_extension_gcode() {
        assert_eq!(
            thumbnail_extension_for_path(&PathBuf::from("print.gcode"))
                .unwrap()
                .to_extension(),
            "gcode"
        );
    }

    #[test]
    fn thumbnail_extension_3mf() {
        assert_eq!(
            thumbnail_extension_for_path(&PathBuf::from("x.3mf"))
                .unwrap()
                .to_extension(),
            "3mf"
        );
    }

    #[test]
    fn thumbnail_extension_stl_zip() {
        assert_eq!(
            thumbnail_extension_for_path(&PathBuf::from("x.stl.zip"))
                .unwrap()
                .to_extension(),
            "stl.zip"
        );
    }

    #[test]
    fn thumbnail_extension_obj_zip() {
        assert_eq!(
            thumbnail_extension_for_path(&PathBuf::from("a/b.obj.zip"))
                .unwrap()
                .to_extension(),
            "obj.zip"
        );
    }

    #[test]
    fn thumbnail_extension_gcode_zip() {
        assert_eq!(
            thumbnail_extension_for_path(&PathBuf::from("out.gcode.zip"))
                .unwrap()
                .to_extension(),
            "gcode.zip"
        );
    }

    #[test]
    fn thumbnail_extension_unknown_returns_err() {
        let err = thumbnail_extension_for_path(&PathBuf::from("x.unknown")).unwrap_err();
        let msg = match &err {
            ServiceError::InternalError(s) => s.as_str(),
            _ => panic!("expected InternalError, got {err:?}"),
        };
        assert!(
            msg.contains("Unsupported file extension for thumbnail generation"),
            "expected error message to mention unsupported extension, got: {msg}"
        );
    }

    #[test]
    fn thumbnail_extension_no_extension_returns_err() {
        let err = thumbnail_extension_for_path(&PathBuf::from("noext")).unwrap_err();
        let msg = match &err {
            ServiceError::InternalError(s) => s.as_str(),
            _ => panic!("expected InternalError, got {err:?}"),
        };
        assert!(
            msg.contains("Unsupported file extension for thumbnail generation"),
            "expected error for path with no extension, got: {msg}"
        );
    }
}
