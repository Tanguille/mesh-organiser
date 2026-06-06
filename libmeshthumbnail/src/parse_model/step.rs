use std::{
    env,
    fs::File,
    io::{self, Write},
    path::Path,
    sync::Mutex,
};

use cadrum::Solid;
use zip::ZipArchive;

use crate::{error::MeshThumbnailError, mesh::Mesh};

/// Override for the scale-independent `Tessellation::default()`. Treating this as an absolute
/// chord is unsafe at arbitrary scales (a 10mm cube with `0.01` would request a 10µm chord and
/// generate hundreds of thousands of triangles); map it to the same relative fraction the
/// upstream default uses so the env var behaves consistently regardless of model size.
const TOLERANCE_DEFAULT: f64 = 0.004;

/// OCCT's mesh kernel keeps process-wide state (`BRepMesh_IncrementalMesh`, FFI handles) that is
/// not safe to invoke from multiple threads concurrently — parallel STEP parses have been
/// observed to segfault inside cadrum's C++ wrapper. Serialize all cadrum calls through this
/// guard; STEP parsing is I/O + CPU bound on a small set of solids per file, so a single
/// in-flight parse is acceptable and avoids a much larger OCCT refactor.
static OCCT_GUARD: Mutex<()> = Mutex::new(());

/// Acquire the OCCT guard and run `f`. Returns the guard's poison error to the caller if a
/// previous holder panicked, rather than transparently swallowing it.
fn with_occt_lock<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let _guard = OCCT_GUARD.lock().expect("OCCT guard poisoned");
    f()
}

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

fn read_and_mesh_step(path: &Path) -> Result<cadrum::Mesh, MeshThumbnailError> {
    let deflection_linear = env::var("LIBMESHTHUMBNAIL_STEP_TRIANGULATION_TOLERANCE")
        .map_or(TOLERANCE_DEFAULT, |value| {
            value.parse::<f64>().unwrap_or(TOLERANCE_DEFAULT)
        });
    let tessellation = cadrum::Tessellation {
        deflection_linear,
        ..cadrum::Tessellation::default()
    };
    let mut file = File::open(path)?;
    let solids = Solid::read_step(&mut file)?;

    Ok(with_occt_lock(|| Solid::mesh(&solids, tessellation))?)
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
