use std::{
    env,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use opencascade::{mesh::Mesher, primitives::Shape};
use stl_io::{Triangle, Vector, Vertex};
use tempfile::TempDir;

use crate::{
    error::MeshThumbnailError,
    mesh::Mesh,
    parse_model::find_zip_entry_bytes,
    path_ext::{is_zip_of, matches_ext},
};

const TOLERANCE_DEFAULT: f64 = 0.01;

pub fn handle_step(path: &Path) -> Result<Option<Mesh>, MeshThumbnailError> {
    if is_zip_of(path, "step") || is_zip_of(path, "stp") {
        Ok(Some(parse_step_zip(path)?))
    } else if matches_ext(path, "step") || matches_ext(path, "stp") {
        Ok(Some(parse_step(path)?))
    } else {
        Ok(None)
    }
}

/// Reads the STEP triangulation tolerance from the
/// `LIBMESHTHUMBNAIL_STEP_TRIANGULATION_TOLERANCE` env var, falling back to
/// `TOLERANCE_DEFAULT` when unset or unparseable.
fn step_tolerance() -> f64 {
    env::var("LIBMESHTHUMBNAIL_STEP_TRIANGULATION_TOLERANCE").map_or(TOLERANCE_DEFAULT, |value| {
        value.parse::<f64>().unwrap_or(TOLERANCE_DEFAULT)
    })
}

/// Stages STEP bytes from `reader` into a temporary `a.step` file. The returned
/// [`TempDir`] must be kept alive for as long as the path is used, since the
/// directory (and file) are removed when it is dropped.
fn stage_step_bytes(reader: &mut impl Read) -> Result<(TempDir, PathBuf), MeshThumbnailError> {
    let temp_dir = tempfile::tempdir()?;
    let mut temp_path = temp_dir.path().to_path_buf();
    temp_path.push("a.step");
    let mut temp_file = File::create(&temp_path)?;
    io::copy(reader, &mut temp_file)?;
    temp_file.flush()?;

    Ok((temp_dir, temp_path))
}

/// f64→f32 truncation: `OpenCascade` gives `f64`; we need `f32` for STL/thumbnails. Precision is
/// sufficient for display and export; explicit rounding would add cost with no practical benefit.
#[allow(clippy::cast_possible_truncation)]
fn parse_step(path: &Path) -> Result<Mesh, MeshThumbnailError> {
    let tolerance = step_tolerance();
    let shape = Shape::read_step(path)?;
    let mesher = Mesher::try_new(&shape, tolerance)?;
    let mesh = mesher.mesh()?;

    Ok(Mesh {
        vertices: mesh
            .vertices
            .into_iter()
            .map(|v| vek::Vec3::new(v.x as f32, v.y as f32, v.z as f32))
            .collect(),
        indices: mesh
            .indices
            .into_iter()
            .map(|i| u32::try_from(i).expect("index too large for u32"))
            .collect(),
    })
}

fn parse_step_zip(path: &Path) -> Result<Mesh, MeshThumbnailError> {
    let buffer = find_zip_entry_bytes(
        path,
        |name| {
            Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("step") || e.eq_ignore_ascii_case("stp"))
        },
        "Failed to find .step model in zip",
    )?;
    // Keep `_temp_dir` alive until after parsing; dropping it removes the staged file.
    let (_temp_dir, temp_path) = stage_step_bytes(&mut buffer.as_slice())?;
    parse_step(&temp_path)
}

/// # Panics
///
/// Panics if unable to create a temporary directory.
///
/// # Errors
///
/// Returns an error if the STEP file cannot be read or meshed, or if writing the STL fails.
pub fn convert_step_to_stl(step: &[u8]) -> Result<Vec<u8>, MeshThumbnailError> {
    // Keep `_temp_dir` alive until after conversion; dropping it removes the staged file.
    let (_temp_dir, temp_path) = stage_step_bytes(&mut &step[..])?;

    convert_step_path_to_stl(&temp_path)
}

/// # Panics
///
/// Panics if unable to create a temporary directory.
///
/// # Errors
///
/// Returns an error if the STEP file cannot be read or meshed, or if writing the STL fails.
///
/// f64→f32: same rationale as `parse_step` (STL/display use f32; truncation is intentional).
#[allow(clippy::cast_possible_truncation)]
pub fn convert_step_path_to_stl(step_path: &Path) -> Result<Vec<u8>, MeshThumbnailError> {
    let tolerance = step_tolerance();
    let shape = Shape::read_step(step_path)?;
    let mesher = Mesher::try_new(&shape, tolerance)?;
    let mesh = mesher.mesh()?;

    let mut triangles: Vec<Triangle> = Vec::with_capacity(mesh.indices.len() / 3 + 1);

    for i in (0..mesh.indices.len()).step_by(3) {
        if i + 2 < mesh.indices.len() {
            let idx0 = mesh.indices[i];
            let idx1 = mesh.indices[i + 1];
            let idx2 = mesh.indices[i + 2];

            let v0 = &mesh.vertices[idx0];
            let v1 = &mesh.vertices[idx1];
            let v2 = &mesh.vertices[idx2];

            let n0 = &mesh.normals[idx0];
            let n1 = &mesh.normals[idx1];
            let n2 = &mesh.normals[idx2];
            let normal = Vector::new([
                ((n0.x + n1.x + n2.x) / 3.0) as f32,
                ((n0.y + n1.y + n2.y) / 3.0) as f32,
                ((n0.z + n1.z + n2.z) / 3.0) as f32,
            ]);

            let triangle = Triangle {
                normal,
                vertices: [
                    Vertex::new([v0.x as f32, v0.y as f32, v0.z as f32]),
                    Vertex::new([v1.x as f32, v1.y as f32, v1.z as f32]),
                    Vertex::new([v2.x as f32, v2.y as f32, v2.z as f32]),
                ],
            };

            triangles.push(triangle);
        }
    }

    let mut data = Vec::new();
    stl_io::write_stl(&mut data, triangles.iter())?;

    Ok(data)
}
