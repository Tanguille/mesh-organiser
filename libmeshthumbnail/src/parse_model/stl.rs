use std::{
    fs::File,
    io::{self, Cursor},
    path::Path,
};

use stl_io::IndexedMesh;
use vek::Vec3;
use zip::ZipArchive;

use crate::{error::MeshThumbnailError, mesh::Mesh};

pub fn handle_stl(path: &Path) -> Result<Option<Mesh>, MeshThumbnailError> {
    let is_stl = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("stl"));
    let is_stl_zip = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        && path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.to_lowercase().ends_with(".stl"));

    if is_stl_zip {
        Ok(Some(parse_stl_zip(path)?))
    } else if is_stl {
        Ok(Some(parse_stl(path)?))
    } else {
        Ok(None)
    }
}

fn parse_stl(path: &Path) -> Result<Mesh, MeshThumbnailError> {
    let mut handle = File::open(path)?;
    let stl = stl_io::read_stl(&mut handle)?;

    Ok(parse_stl_inner(&stl))
}

fn parse_stl_zip(path: &Path) -> Result<Mesh, MeshThumbnailError> {
    let handle = File::open(path)?;
    let mut zip = ZipArchive::new(handle)?;

    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        if std::path::Path::new(file.name())
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("stl"))
        {
            let mut buffer = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0));
            io::copy(&mut file, &mut buffer)?;
            let mut cursor = Cursor::new(buffer);

            let stl = stl_io::read_stl(&mut cursor)?;
            return Ok(parse_stl_inner(&stl));
        }
    }

    Err(MeshThumbnailError::InternalError(String::from(
        "Failed to find .stl model in zip",
    )))
}

// https://github.com/asny/three-d-asset/blob/main/src/io/stl.rs#L9
fn parse_stl_inner(stl: &IndexedMesh) -> Mesh {
    let vertices: Vec<Vec3<f32>> = stl
        .vertices
        .iter()
        .map(|v| Vec3::new(v[0], v[1], v[2]))
        .collect();

    let indices: Vec<u32> = stl
        .faces
        .iter()
        .flat_map(|face| {
            face.vertices
                .map(|idx| u32::try_from(idx).unwrap_or(0))
                .into_iter()
        })
        .collect();

    Mesh { vertices, indices }
}
