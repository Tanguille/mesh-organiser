use std::{collections::HashMap, fs::File, io::Read, path::Path};

use vek::Vec3;
use wavefront_obj::obj::{self, ObjSet};
use zip::ZipArchive;

use crate::{error::MeshThumbnailError, mesh::Mesh};

pub fn handle(path: &Path) -> Result<Option<Mesh>, MeshThumbnailError> {
    let stem = path.file_stem().and_then(|s| s.to_str());
    let is_obj = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("obj"));
    let is_obj_zip = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        && stem.is_some_and(|s| {
            Path::new(s)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("obj"))
        });

    if is_obj_zip {
        Ok(Some(parse_zip(path)?))
    } else if is_obj {
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
    let handle = File::open(path)?;
    let mut zip = ZipArchive::new(handle)?;

    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        if Path::new(file.name())
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("obj"))
        {
            let mut buffer = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0));
            file.read_to_end(&mut buffer)?;

            let utf8 = std::str::from_utf8(&buffer).map_err(|e| {
                MeshThumbnailError::InternalError(format!("OBJ content is not valid UTF-8: {e}"))
            })?;
            return parse_inner(&obj::parse(utf8)?);
        }
    }

    Err(MeshThumbnailError::InternalError(String::from(
        "Failed to find .obj model in zip",
    )))
}

// https://github.com/asny/three-d-asset/blob/main/src/io/obj.rs#L54
fn parse_inner(obj: &ObjSet) -> Result<Mesh, MeshThumbnailError> {
    let mut all_meshes: Vec<Mesh> = obj
        .objects
        .iter()
        .map(|object| {
            let mut positions = Vec::new();
            let mut indices = Vec::new();
            for mesh in &object.geometry {
                let mut map: HashMap<usize, usize> = HashMap::new();

                let mut process = |i: obj::VTNIndex| {
                    let mut index = map.get(&i.0).copied();

                    if index.is_none() {
                        index = Some(positions.len());
                        map.insert(i.0, index.unwrap());
                        let position = object.vertices[i.0];
                        // OBJ vertex coordinates are typically in a range that fits in f32; mesh data is not high-precision.
                        #[allow(clippy::cast_possible_truncation)]
                        positions.push(Vec3::new(
                            position.x as f32,
                            position.y as f32,
                            position.z as f32,
                        ));
                    }

                    indices.push(u32::try_from(index.unwrap()).unwrap_or(0));
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
        .collect();

    all_meshes.sort_by_key(|a| a.indices.len());

    if all_meshes.is_empty() {
        return Err(MeshThumbnailError::InternalError(String::from(
            "No meshes found in obj model",
        )));
    }

    let mesh = all_meshes.pop().unwrap();

    Ok(mesh)
}
