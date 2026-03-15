use std::{
    env,
    fs::File,
    io::{self, Write},
    path::Path,
};

use opencascade::{mesh::Mesher, primitives::Shape};
use stl_io::{Triangle, Vector, Vertex};
use zip::ZipArchive;

use crate::{error::MeshThumbnailError, mesh::Mesh};

const TOLERANCE_DEFAULT: f64 = 0.01;

pub fn handle_step(path: &Path) -> Result<Option<Mesh>, MeshThumbnailError> {
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let lower_file_name = file_name.to_lowercase();

    if lower_file_name.ends_with(".step.zip") || lower_file_name.ends_with(".stp.zip") {
        Ok(Some(parse_step_zip(path)?))
    } else {
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if extension == "step" || extension == "stp" {
            Ok(Some(parse_step(path)?))
        } else {
            Ok(None)
        }
    }
}

/// f64→f32 truncation: `OpenCascade` gives `f64`; we need `f32` for STL/thumbnails. Precision is
/// sufficient for display and export; explicit rounding would add cost with no practical benefit.
#[allow(clippy::cast_possible_truncation)]
fn parse_step(path: &Path) -> Result<Mesh, MeshThumbnailError> {
    let tolerance = env::var("LIBMESHTHUMBNAIL_STEP_TRIANGULATION_TOLERANCE")
        .map(|val| val.parse::<f64>().unwrap_or(TOLERANCE_DEFAULT))
        .unwrap_or(TOLERANCE_DEFAULT);
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
    let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
    let mut temp_path = temp_dir.path().to_path_buf();
    temp_path.push("a.step");
    let mut temp_file = File::create(&temp_path)?;
    let handle = File::open(path)?;
    let mut zip = ZipArchive::new(handle)?;
    let mut write_ok = false;

    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        let file_name = file.name();
        let ext_matches = Path::new(file_name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("step") || ext.eq_ignore_ascii_case("stp"));
        if ext_matches {
            io::copy(&mut file, &mut temp_file)?;
            temp_file.flush()?;
            write_ok = true;
            break;
        }
    }

    drop(zip);
    drop(temp_file);

    if !write_ok {
        return Err(MeshThumbnailError::InternalError(String::from(
            "Failed to find .step model in zip",
        )));
    }

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
    // Todo: This is kinda hacky
    let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");
    let mut temp_path = temp_dir.path().to_path_buf();
    temp_path.push("a.step");
    let mut temp_file = File::create(&temp_path)?;
    temp_file.write_all(step)?;
    temp_file.flush()?;
    drop(temp_file);

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
    let tolerance = env::var("LIBMESHTHUMBNAIL_STEP_TRIANGULATION_TOLERANCE")
        .map(|val| val.parse::<f64>().unwrap_or(TOLERANCE_DEFAULT))
        .unwrap_or(TOLERANCE_DEFAULT);
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
