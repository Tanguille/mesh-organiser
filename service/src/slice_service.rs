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
//! `{binary} --help` on your install. Per-request settings (layer height, infill, ...) are not
//! accepted until a CLI mapping is verified on real installs; the slicer's own defaults apply.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use db::{model::user::User, model_db};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::{
    export_service::get_path_from_model, import_service::import_single_model,
    service_error::ServiceError, util::cleanse_evil_from_name,
};

use super::app_state::AppState;

/// Environment variable: absolute path to `OrcaSlicer` / Prusa-family **console** executable.
pub const ORCA_SLICER_EXECUTABLE_ENV: &str = "MESH_ORGANISER_ORCA_PATH";

/// Subprocess wall-clock limit (large models can be slow).
const SLICE_TIMEOUT: Duration = Duration::from_secs(600);

/// Result of a successful slice: new model row pointing at stored g-code blob.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SliceOrchestrationResult {
    pub output_model_id: i64,
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

async fn find_gcode_in_output_dir(dir: &Path) -> Result<PathBuf, ServiceError> {
    // Lowest path wins so the pick is deterministic when the slicer emits several plates.
    let mut entries = tokio::fs::read_dir(dir).await?;
    let mut lowest: Option<PathBuf> = None;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let is_gcode = path.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("gcode") || extension.eq_ignore_ascii_case("g")
        });
        if is_gcode && lowest.as_ref().is_none_or(|current| path < *current) {
            lowest = Some(path);
        }
    }

    lowest.ok_or_else(|| {
        ServiceError::InternalError(
            "Slicer finished but no .gcode file was found in the output directory".to_string(),
        )
    })
}

/// Stores slice output as a new **gcode** model via the regular import path (hash dedupe, zipped on disk).
async fn persist_gcode_as_new_model(
    file_contents: Vec<u8>,
    source_model_name: &str,
    app_state: &AppState,
    user: &User,
) -> Result<SliceOrchestrationResult, ServiceError> {
    let name = format!("{} (slice)", cleanse_evil_from_name(source_model_name));
    let output_model_id = import_single_model(
        &mut file_contents.as_slice(),
        "gcode",
        file_contents.len(),
        &name,
        None,
        app_state,
        user,
        None,
    )
    .await?;

    Ok(SliceOrchestrationResult { output_model_id })
}

/// Runs the external slicer; returns the path to the primary `.gcode` file.
async fn run_slicer_subprocess(
    slicer: &Path,
    work_dir: &Path,
    input_path: &Path,
    out_dir: &Path,
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

    find_gcode_in_output_dir(out_dir).await
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
) -> Result<SliceOrchestrationResult, ServiceError> {
    let models = model_db::get_models_via_ids(&app_state.db, user, vec![model_id]).await?;
    let Some(model) = models.into_iter().next() else {
        return Err(ServiceError::InternalError(format!(
            "Model not found or not accessible: model_id={model_id}"
        )));
    };

    let file_type = model.blob.to_file_type();
    if file_type.is_gcode() || file_type.is_unsupported() {
        return Err(ServiceError::InternalError(format!(
            "Model file type cannot be sliced: {file_type:?}"
        )));
    }

    let slicer = resolve_slicer_executable_path()?;

    let work_dir = crate::export_service::get_temp_dir("slice");
    let out_dir = work_dir.join("slice_out");

    // Lazy: zipped blobs are still extracted into `work_dir`; others are read in place from the
    // model dir / disk path, which is safe because cleanup below only removes `work_dir`.
    let input_path = get_path_from_model(&work_dir, &model, app_state, true).await?;
    let gcode_path = run_slicer_subprocess(&slicer, &work_dir, &input_path, &out_dir).await?;
    let file_contents = tokio::fs::read(&gcode_path).await?;

    let result = persist_gcode_as_new_model(file_contents, &model.name, app_state, user).await;

    let _ = tokio::fs::remove_dir_all(&work_dir).await;

    result
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use db::db_context;
    use tempfile::tempdir;

    use crate::{configuration::Configuration, util::convert_extension_to_zip};

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

        let err = slice_model_for_user(&app_state, &user, 999_999)
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

        let picked = find_gcode_in_output_dir(dir.path()).await.unwrap();
        assert_eq!(picked.file_name().unwrap(), "a.gcode");
    }

    #[tokio::test]
    async fn persist_gcode_creates_zipped_model_and_dedupes_by_content() {
        let (_dir, app_state) = app_state_with_db().await;
        let user = User::default();
        let gcode_bytes = b"G1 X0 Y0\nG1 X10 Y10\n".to_vec();

        let first = persist_gcode_as_new_model(gcode_bytes.clone(), "Benchy", &app_state, &user)
            .await
            .unwrap();
        let models =
            model_db::get_models_via_ids(&app_state.db, &user, vec![first.output_model_id])
                .await
                .unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "Benchy (slice)");
        assert_eq!(models[0].blob.filetype, convert_extension_to_zip("gcode"));
        assert_eq!(
            model_db::get_model_count(&app_state.db, &user, None)
                .await
                .unwrap(),
            1
        );

        let second = persist_gcode_as_new_model(gcode_bytes, "Other", &app_state, &user)
            .await
            .unwrap();

        assert_eq!(second.output_model_id, first.output_model_id);
        assert_eq!(
            model_db::get_model_count(&app_state.db, &user, None)
                .await
                .unwrap(),
            1
        );
    }
}
