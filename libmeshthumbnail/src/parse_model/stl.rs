use std::{fs::File, io::Cursor, path::Path};

use stl_io::IndexedMesh;
use vek::Vec3;

use crate::{
    error::MeshThumbnailError,
    mesh::Mesh,
    parse_model::find_zip_entry_bytes,
    path_ext::{is_zip_of, matches_ext},
};

pub fn handle_stl(path: &Path) -> Result<Option<Mesh>, MeshThumbnailError> {
    if is_zip_of(path, "stl") {
        Ok(Some(parse_stl_zip(path)?))
    } else if matches_ext(path, "stl") {
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
    let buffer = find_zip_entry_bytes(
        path,
        |name| {
            Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("stl"))
        },
        "Failed to find .stl model in zip",
    )?;
    let mut cursor = Cursor::new(buffer);

    let stl = stl_io::read_stl(&mut cursor)?;

    Ok(parse_stl_inner(&stl))
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
        .flat_map(|face| face.vertices.map(|idx| u32::try_from(idx).unwrap_or(0)))
        .collect();

    Mesh { vertices, indices }
}
