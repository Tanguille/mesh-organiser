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

    let mut all_meshes: Vec<&threemf::Mesh> = mfmodel
        .iter()
        .flat_map(|f| f.resources.object.iter())
        .filter_map(|f| f.mesh.as_ref())
        .collect();

    all_meshes.sort_by(|a, b| {
        a.triangles
            .triangle
            .len()
            .cmp(&b.triangles.triangle.len())
            .reverse()
    });

    if all_meshes.is_empty() {
        return Err(MeshThumbnailError::InternalError(String::from(
            "No meshes found in 3mf model",
        )));
    }

    let mesh = all_meshes[0];

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
