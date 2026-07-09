use std::{collections::HashMap, fs::File, io::Read, path::Path};

use vek::Vec3;
use wavefront_obj::obj::{self, ObjSet};

use crate::{
    error::MeshThumbnailError,
    mesh::Mesh,
    parse_model::find_zip_entry_bytes,
    path_ext::{is_zip_of, matches_ext},
};

pub fn handle(path: &Path) -> Result<Option<Mesh>, MeshThumbnailError> {
    if is_zip_of(path, "obj") {
        Ok(Some(parse_zip(path)?))
    } else if matches_ext(path, "obj") {
        Ok(Some(parse(path)?))
    } else {
        Ok(None)
    }
}

fn parse(path: &Path) -> Result<Mesh, MeshThumbnailError> {
    let mut handle = File::open(path)?;
    let mut buffer = Vec::new();
    handle.read_to_end(&mut buffer)?;

    let utf8 = std::str::from_utf8(&buffer).map_err(|e| {
        MeshThumbnailError::InternalError(format!("OBJ content is not valid UTF-8: {e}"))
    })?;
    let obj = obj::parse(utf8)?;
    parse_inner(&obj)
}

fn parse_zip(path: &Path) -> Result<Mesh, MeshThumbnailError> {
    let buffer = find_zip_entry_bytes(
        path,
        |name| matches_ext(Path::new(name), "obj"),
        "Failed to find .obj model in zip",
    )?;

    let utf8 = std::str::from_utf8(&buffer).map_err(|e| {
        MeshThumbnailError::InternalError(format!("OBJ content is not valid UTF-8: {e}"))
    })?;

    parse_inner(&obj::parse(utf8)?)
}

// https://github.com/asny/three-d-asset/blob/main/src/io/obj.rs#L54
fn parse_inner(obj: &ObjSet) -> Result<Mesh, MeshThumbnailError> {
    obj.objects
        .iter()
        .map(|object| {
            let mut positions = Vec::new();
            let mut indices = Vec::new();
            for mesh in &object.geometry {
                let mut map: HashMap<usize, usize> = HashMap::new();

                let mut process = |i: obj::VTNIndex| {
                    let index = *map.entry(i.0).or_insert_with(|| {
                        let new_index = positions.len();
                        let position = object.vertices[i.0];
                        // OBJ vertex coordinates are typically in a range that fits in f32; mesh data is not high-precision.
                        #[allow(clippy::cast_possible_truncation)]
                        positions.push(Vec3::new(
                            position.x as f32,
                            position.y as f32,
                            position.z as f32,
                        ));
                        new_index
                    });

                    indices.push(u32::try_from(index).unwrap_or(0));
                };
                for shape in &mesh.shapes {
                    // All triangles with same material
                    if let obj::Primitive::Triangle(i0, i1, i2) = shape.primitive {
                        process(i0);
                        process(i1);
                        process(i2);
                    }
                }
            }

            Mesh {
                vertices: positions,
                indices,
            }
        })
        .max_by_key(|m| m.indices.len())
        .ok_or_else(|| {
            MeshThumbnailError::InternalError(String::from("No meshes found in obj model"))
        })
}
