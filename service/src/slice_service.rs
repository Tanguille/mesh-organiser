//! Server-side slice orchestration for the remote **`web`** API (Task 3 will call into this module).
//!
//! ## Slicer executable
//!
//! Set **`MESH_ORGANISER_ORCA_PATH`** to the path of an **`OrcaSlicer`** or **PrusaSlicer-family** console
//! binary (on Windows prefer `orca-slicer-console.exe` when available so stdout/stderr work).
//!
//! ## CLI invocation (v1)
//!
//! The subprocess is invoked roughly as:
//!
//! ```text
//! {MESH_ORGANISER_ORCA_PATH} --slice 0 --outputdir <dir> <input_model_path>
//! ```
//!
//! Exact flags and optional `--load-settings` / `--datadir` behaviour are **product-dependent**; see
//! [OrcaSlicer CLI discussions](https://github.com/SoftFever/OrcaSlicer/discussions) and run
//! `{binary} --help` on your install. [`SliceOrchestrationSettings`] fields are reserved for mapping
//! to CLI arguments once confirmed on real installs.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use db::{
    blob_db,
    model::{blob::FileType, user::User},
    model_db,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::Command;

use crate::{
    export_service::get_path_from_model,
    service_error::ServiceError,
    util::{cleanse_evil_from_name, convert_extension_to_zip, is_zippable_file_extension},
};

use super::app_state::AppState;

/// Environment variable: absolute path to `OrcaSlicer` / Prusa-family **console** executable.
pub const ORCA_SLICER_EXECUTABLE_ENV: &str = "MESH_ORGANISER_ORCA_PATH";

/// Subprocess wall-clock limit (large models can be slow).
const SLICE_TIMEOUT: Duration = Duration::from_secs(600);

/// Basic slicing options for HTTP / JSON (Orca CLI mapping is partial in v1).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SliceOrchestrationSettings {
    /// Layer height in millimetres (CLI mapping **TBD**).
    pub layer_height_mm: Option<f64>,
    /// Infill percentage 0–100 (CLI mapping **TBD**).
    pub infill_percent: Option<u8>,
}

/// Result of a successful slice: new model row pointing at stored g-code blob.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SliceOrchestrationResult {
    pub output_model_id: i64,
    pub output_blob_sha256: String,
}

const fn apply_slice_settings_to_command(
    _cmd: &mut Command,
    _settings: &SliceOrchestrationSettings,
) {
    // TBD: map `layer_height_mm`, `infill_percent`, etc. to verified Orca/Prusa CLI flags.
}

/// Resolves the slicer binary from [`ORCA_SLICER_EXECUTABLE_ENV`].
fn resolve_slicer_executable_path() -> Result<PathBuf, ServiceError> {
    let raw = std::env::var_os(ORCA_SLICER_EXECUTABLE_ENV).ok_or_else(|| {
        ServiceError::InternalError(format!(
            "Slicer not configured: set {ORCA_SLICER_EXECUTABLE_ENV} to the OrcaSlicer (or Prusa-family console) executable path"
        ))
    })?;

    if raw.is_empty() {
        return Err(ServiceError::InternalError(format!(
            "Slicer not configured: {ORCA_SLICER_EXECUTABLE_ENV} is empty"
        )));
    }

    let path = PathBuf::from(raw);
    if !path.exists() {
        return Err(ServiceError::InternalError(format!(
            "Slicer executable not found: {}",
            path.display()
        )));
    }

    Ok(path)
}

const fn model_is_sliceable(file_type: &FileType) -> bool {
    matches!(
        file_type,
        &FileType::Stl
            | &FileType::ZippedStl
            | &FileType::Obj
            | &FileType::ZippedObj
            | &FileType::Threemf
            | &FileType::Step
            | &FileType::ZippedStep
    )
}

fn find_gcode_in_output_dir(dir: &Path) -> Result<PathBuf, ServiceError> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|e| {
            e.path().extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("gcode") || ext.eq_ignore_ascii_case("g")
            })
        })
        .map(|e| e.path())
        .collect();

    if candidates.is_empty() {
        return Err(ServiceError::InternalError(
            "Slicer finished but no .gcode file was found in the output directory".to_string(),
        ));
    }

    candidates.sort();

    Ok(candidates.remove(0))
}

fn mesh_asset_sha256_prefix_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hash = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        let _ = write!(hash, "{byte:02x}");
    }

    hash
}

/// Writes slice output bytes as a **gcode** blob and model row (same storage rules as import: zip on disk when zippable).
async fn persist_gcode_as_new_model(
    file_contents: Vec<u8>,
    source_model_name: &str,
    app_state: &AppState,
    user: &User,
) -> Result<SliceOrchestrationResult, ServiceError> {
    let file_size = file_contents.len();
    let hash = mesh_asset_sha256_prefix_hex(&file_contents);

    let existing_id = model_db::get_model_id_via_sha256(&app_state.db, user, &hash).await?;
    if let Some(model_id) = existing_id {
        return Ok(SliceOrchestrationResult {
            output_model_id: model_id,
            output_blob_sha256: hash,
        });
    }

    let file_type = "gcode";
    let blob_id_optional = blob_db::get_blob_via_sha256(&app_state.db, &hash).await?;

    let blob_id = if let Some(blob) = blob_id_optional {
        blob.id
    } else {
        let new_extension = convert_extension_to_zip(file_type);
        let final_file_name = app_state
            .get_model_dir()
            .join(format!("{hash}.{new_extension}"));

        let mut file_handle = tokio::fs::File::create(&final_file_name).await?;

        if is_zippable_file_extension(file_type) {
            use async_zip::{Compression, ZipEntryBuilder, tokio::write::ZipFileWriter};
            let mut writer = ZipFileWriter::with_tokio(&mut file_handle);
            let builder = ZipEntryBuilder::new(
                format!(
                    "{}.{}",
                    cleanse_evil_from_name(source_model_name),
                    file_type
                )
                .into(),
                Compression::Deflate,
            );
            writer.write_entry_whole(builder, &file_contents).await?;
            writer.close().await?;
        } else {
            tokio::io::AsyncWriteExt::write_all(&mut file_handle, &file_contents).await?;
        }

        blob_db::add_blob(
            &app_state.db,
            &hash,
            &new_extension,
            i64::try_from(file_size).unwrap_or(i64::MAX),
            None,
        )
        .await?
    };

    let name = format!("{} (slice)", cleanse_evil_from_name(source_model_name));
    let model_id = model_db::add_model(&app_state.db, user, &name, blob_id, None, None).await?;

    Ok(SliceOrchestrationResult {
        output_model_id: model_id,
        output_blob_sha256: hash,
    })
}

/// Runs the external slicer; returns the path to the primary `.gcode` file.
async fn run_slicer_subprocess(
    slicer: &Path,
    work_dir: &Path,
    input_path: &Path,
    out_dir: &Path,
    settings: &SliceOrchestrationSettings,
) -> Result<PathBuf, ServiceError> {
    tokio::fs::create_dir_all(out_dir).await?;

    let mut cmd = Command::new(slicer);
    cmd.kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(work_dir)
        .arg("--slice")
        .arg("0")
        .arg("--outputdir")
        .arg(out_dir)
        .arg(input_path);

    apply_slice_settings_to_command(&mut cmd, settings);

    let output = tokio::time::timeout(SLICE_TIMEOUT, cmd.output())
        .await
        .map_err(|_| {
            ServiceError::InternalError(format!(
                "Slicer timed out after {}s",
                SLICE_TIMEOUT.as_secs()
            ))
        })?
        .map_err(ServiceError::FileSystemFault)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        return Err(ServiceError::InternalError(format!(
            "Slicer exited with status {}: stderr={stderr} stdout={stdout}",
            output.status
        )));
    }

    find_gcode_in_output_dir(out_dir)
}

/// Resolves the model, runs the configured slicer, stores g-code as a new model.
///
/// # Errors
///
/// Returns [`ServiceError`] if the model is missing, the input is not sliceable, the slicer is
/// misconfigured, the subprocess fails, or persistence fails.
pub async fn slice_model_for_user(
    app_state: &AppState,
    user: &User,
    model_id: i64,
    settings: &SliceOrchestrationSettings,
) -> Result<SliceOrchestrationResult, ServiceError> {
    let models = model_db::get_models_via_ids(&app_state.db, user, vec![model_id]).await?;
    let Some(model) = models.into_iter().next() else {
        return Err(ServiceError::InternalError(format!(
            "Model not found or not accessible: model_id={model_id}"
        )));
    };

    let file_type = model.blob.to_file_type();
    if !model_is_sliceable(&file_type) {
        return Err(ServiceError::InternalError(format!(
            "Model file type cannot be sliced: {file_type:?}"
        )));
    }

    let slicer = resolve_slicer_executable_path()?;

    let work_dir = crate::export_service::get_temp_dir("slice");
    let out_dir = work_dir.join("slice_out");

    let input_path = get_path_from_model(&work_dir, &model, app_state, false).await?;
    let gcode_path =
        run_slicer_subprocess(&slicer, &work_dir, &input_path, &out_dir, settings).await?;
    let file_contents = tokio::fs::read(&gcode_path).await?;

    let result = persist_gcode_as_new_model(file_contents, &model.name, app_state, user).await;

    let _ = std::fs::remove_dir_all(&work_dir);

    result
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use db::db_context;
    use tempfile::tempdir;

    use crate::configuration::Configuration;

    use super::*;

    async fn app_state_with_db() -> (tempfile::TempDir, AppState) {
        let dir = tempdir().unwrap();
        let data_path = dir.path().join("data");
        std::fs::create_dir_all(&data_path).unwrap();
        let db_path = data_path.join("db.sqlite");
        let backup_dir = data_path.join("backup");
        std::fs::create_dir_all(&backup_dir).unwrap();

        let db = db_context::setup_db(&db_path, &backup_dir).await;
        let config = Configuration {
            data_path: data_path.to_string_lossy().to_string(),
            ..Default::default()
        };

        let app_state = AppState {
            db: Arc::new(db),
            configuration: Mutex::new(config),
            import_mutex: Arc::new(tokio::sync::Mutex::new(())),
            app_data_path: data_path.to_string_lossy().to_string(),
        };

        (dir, app_state)
    }

    #[tokio::test]
    async fn slice_rejects_unknown_model() {
        let (_dir, app_state) = app_state_with_db().await;
        let user = User::default();
        let settings = SliceOrchestrationSettings::default();

        let err = slice_model_for_user(&app_state, &user, 999_999, &settings)
            .await
            .expect_err("unknown model_id should error");

        let msg = err.to_string();
        assert!(
            msg.contains("Model not found")
                || msg.contains("not accessible")
                || msg.contains("Internal error"),
            "unexpected message: {msg}"
        );
    }

    #[tokio::test]
    async fn find_gcode_in_output_dir_prefers_single_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.gcode"), b";a").unwrap();
        std::fs::write(dir.path().join("b.gcode"), b";b").unwrap();

        let picked = find_gcode_in_output_dir(dir.path()).unwrap();
        assert_eq!(picked.file_name().unwrap(), "a.gcode");
    }
}
