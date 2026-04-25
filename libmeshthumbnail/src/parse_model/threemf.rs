use std::path::Path;

use vek::Vec3;

use crate::{error::MeshThumbnailError, mesh::Mesh};

pub fn handle_threemf(path: &Path) -> Result<Option<Mesh>, MeshThumbnailError> {
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"))
    {
        Ok(Some(parse_3mf(path)?))
    } else {
        Ok(None)
    }
}

fn parse_3mf(path: &Path) -> Result<Mesh, MeshThumbnailError> {
    let handle = std::fs::File::open(path)?;
    let mfmodel = threemf::read(handle)?;

    let mesh = mfmodel
        .iter()
        .flat_map(|f| f.resources.object.iter())
        .filter_map(|f| f.mesh.as_ref())
        .max_by_key(|m| m.triangles.triangle.len())
        .ok_or_else(|| {
            MeshThumbnailError::InternalError(String::from("No meshes found in 3mf model"))
        })?;

    // 3MF vertex coordinates are mesh-scale; f64→f32 truncation is acceptable for thumbnail rendering.
    #[allow(clippy::cast_possible_truncation)]
    let positions = mesh
        .vertices
        .vertex
        .iter()
        .map(|a| Vec3 {
            x: a.x as f32,
            y: a.y as f32,
            z: a.z as f32,
        })
        .collect();

    let indices = mesh
        .triangles
        .triangle
        .iter()
        .flat_map(|a| {
            [
                u32::try_from(a.v1).unwrap_or(0),
                u32::try_from(a.v2).unwrap_or(0),
                u32::try_from(a.v3).unwrap_or(0),
            ]
            .into_iter()
        })
        .collect();

    Ok(Mesh {
        vertices: positions,
        indices,
    })
}
