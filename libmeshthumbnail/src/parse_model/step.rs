use std::{
    env,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use cadrum::Solid;
use tempfile::TempDir;

use crate::{
    error::MeshThumbnailError,
    mesh::Mesh,
    parse_model::copy_zip_entry_to,
    path_ext::{is_zip_of, matches_ext},
};

/// Override for the scale-independent `Tessellation::default()`. Treating this as an absolute
/// chord is unsafe at arbitrary scales (a 10mm cube with `0.01` would request a 10µm chord and
/// generate hundreds of thousands of triangles); map it to the same relative fraction the
/// upstream default uses so the env var behaves consistently regardless of model size.
const TOLERANCE_DEFAULT: f64 = 0.004;

/// OCCT's mesh kernel keeps process-wide state (`BRepMesh_IncrementalMesh`, FFI handles) that is
/// not safe to invoke from multiple threads concurrently — parallel STEP parses have been
/// observed to segfault inside cadrum's C++ wrapper. Serialize all cadrum meshing through this
/// guard; STEP parsing is I/O + CPU bound on a small set of solids per file, so a single
/// in-flight parse is acceptable and avoids a much larger OCCT refactor.
static OCCT_GUARD: Mutex<()> = Mutex::new(());

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

fn read_and_mesh_step(path: &Path) -> Result<cadrum::Mesh, MeshThumbnailError> {
    let tessellation = cadrum::Tessellation {
        deflection_linear: step_tolerance(),
        ..cadrum::Tessellation::default()
    };
    let mut file = File::open(path)?;
    let solids = Solid::read_step(&mut file)?;

    // Poison propagates as a panic on purpose: it means a previous mesh call panicked.
    let _guard = OCCT_GUARD.lock().expect("OCCT guard poisoned");
    Ok(Solid::mesh(&solids, tessellation)?)
}

/// f64→f32 truncation: OCCT gives `f64`; we need `f32` for STL/thumbnails. Precision is
/// sufficient for display and export; explicit rounding would add cost with no practical benefit.
#[allow(clippy::cast_possible_truncation)]
fn parse_step(path: &Path) -> Result<Mesh, MeshThumbnailError> {
    let mesh = read_and_mesh_step(path)?;

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
    // Stream the entry straight to disk: decompressed STEP files can be large,
    // so avoid buffering them fully in memory.
    let temp_dir = tempfile::tempdir()?;
    let temp_path = temp_dir.path().join("a.step");
    let mut temp_file = File::create(&temp_path)?;
    copy_zip_entry_to(
        path,
        |name| {
            let name = Path::new(name);
            matches_ext(name, "step") || matches_ext(name, "stp")
        },
        "Failed to find .step model in zip",
        &mut temp_file,
    )?;
    temp_file.flush()?;
    drop(temp_file);
    // `temp_dir` stays alive until after parsing; dropping it removes the staged file.
    parse_step(&temp_path)
}

/// # Errors
///
/// Returns an error if staging the bytes fails, or if the STEP file cannot be
/// read, meshed, or written as STL.
pub fn convert_step_to_stl(step: &[u8]) -> Result<Vec<u8>, MeshThumbnailError> {
    // Keep `_temp_dir` alive until after conversion; dropping it removes the staged file.
    let (_temp_dir, temp_path) = stage_step_bytes(&mut &step[..])?;

    convert_step_path_to_stl(&temp_path)
}

/// # Errors
///
/// Returns an error if the STEP file cannot be read or meshed, or if writing the STL fails.
pub fn convert_step_path_to_stl(step_path: &Path) -> Result<Vec<u8>, MeshThumbnailError> {
    let mesh = read_and_mesh_step(step_path)?;
    let mut data = Vec::new();
    mesh.write_stl(&mut data)?;

    Ok(data)
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write as _, path::PathBuf};

    use tempfile::tempdir;
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::{convert_step_path_to_stl, handle_step};

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cube.step")
    }

    #[test]
    fn convert_step_path_to_stl_produces_binary_stl() {
        let stl = convert_step_path_to_stl(&fixture_path()).expect("cube.step should convert");

        assert!(
            stl.len() > 84,
            "binary STL should include header and triangles"
        );
        assert!(!stl.is_empty());
    }

    #[test]
    fn handle_step_parses_cube_fixture_into_mesh() {
        let mesh = handle_step(&fixture_path())
            .expect("cube.step should parse")
            .expect("step file should be recognized");

        assert!(!mesh.vertices.is_empty());
        assert!(!mesh.indices.is_empty());
        assert_eq!(mesh.indices.len() % 3, 0, "indices should form triangles");
    }

    #[test]
    fn handle_step_supports_zipped_step_fixture() {
        let temp_dir = tempdir().expect("temp dir");
        let zip_path = temp_dir.path().join("cube.step.zip");
        let mut zip = ZipWriter::new(File::create(&zip_path).expect("zip file"));
        let options = SimpleFileOptions::default();
        let step_bytes = std::fs::read(fixture_path()).expect("read fixture");

        zip.start_file("cube.step", options).expect("zip entry");
        zip.write_all(&step_bytes).expect("write fixture");
        zip.finish().expect("finish zip");

        let mesh = handle_step(&zip_path)
            .expect("zipped cube.step should parse")
            .expect("zipped step file should be recognized");

        assert!(!mesh.vertices.is_empty());
        assert!(!mesh.indices.is_empty());
    }
}
