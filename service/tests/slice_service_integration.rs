//! Integration tests for [`service::slice_service`] (no real `OrcaSlicer` in CI).

use db::{blob_db, model::user::User, model_db};
use service::{
    AppState,
    slice_service::{ORCA_SLICER_EXECUTABLE_ENV, slice_model_for_user},
};
use tempfile::tempdir;

mod common;

use common::{internal_error_message, test_app_state};

async fn app_state_with_stl_model() -> (tempfile::TempDir, AppState, i64) {
    let (dir, app_state) = test_app_state().await;

    let model_dir = app_state.get_model_dir();
    let sha256 = "0123456789abcdef0123456789abcdef";
    let filetype = "stl";
    let content = b"solid mock\nendsolid mock\n";
    let path = model_dir.join(format!("{sha256}.{filetype}"));
    std::fs::write(&path, content).unwrap();

    let blob_id = blob_db::add_blob(
        &app_state.db,
        sha256,
        filetype,
        i64::try_from(content.len()).unwrap(),
        None,
    )
    .await
    .unwrap();

    let user = User::default();
    let model_id = model_db::add_model(&app_state.db, &user, "MockPart", blob_id, None, None)
        .await
        .unwrap();

    (dir, app_state, model_id)
}

/// Writes a stand-in slicer that puts `;sliced` into `<outputdir>/plate_1.gcode`, so the test
/// fails if the `--outputdir` wiring or the g-code handoff breaks.
#[cfg(unix)]
fn fake_slicer(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("fake-orca");
    std::fs::write(
        &path,
        "#!/bin/sh\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = --outputdir ]; then out=\"$2\"; fi\n  shift\ndone\nprintf ';sliced\\n' > \"$out/plate_1.gcode\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

    path
}

// All cases mutate the process-wide slicer env var, so they live in one test: separate tests run
// on parallel threads and would race on it.
#[tokio::test]
async fn slice_with_unset_missing_and_working_slicer() {
    let prior = std::env::var(ORCA_SLICER_EXECUTABLE_ENV).ok();
    let (_dir, app_state, model_id) = app_state_with_stl_model().await;
    let user = User::default();

    // SAFETY: no other test in this binary touches this env var.
    unsafe { std::env::remove_var(ORCA_SLICER_EXECUTABLE_ENV) };

    let unset_message =
        internal_error_message(slice_model_for_user(&app_state, &user, model_id).await);
    assert!(
        unset_message.starts_with("Slicer not configured"),
        "unexpected message: {unset_message}"
    );

    let slicer_dir = tempdir().unwrap();
    let ghost = slicer_dir.path().join("nonexistent_orca_binary.exe");
    // SAFETY: no other test in this binary touches this env var.
    unsafe { std::env::set_var(ORCA_SLICER_EXECUTABLE_ENV, ghost.as_os_str()) };

    let missing_message =
        internal_error_message(slice_model_for_user(&app_state, &user, model_id).await);
    assert!(
        missing_message.starts_with("Slicer executable not found"),
        "unexpected message: {missing_message}"
    );

    #[cfg(unix)]
    {
        // SAFETY: no other test in this binary touches this env var.
        unsafe {
            std::env::set_var(ORCA_SLICER_EXECUTABLE_ENV, fake_slicer(slicer_dir.path()));
        };

        let result = slice_model_for_user(&app_state, &user, model_id)
            .await
            .expect("fake slicer run should persist its g-code");
        let models =
            model_db::get_models_via_ids(&app_state.db, &user, vec![result.output_model_id])
                .await
                .unwrap();
        assert_eq!(models[0].name, "MockPart (slice)");
        assert!(models[0].blob.to_file_type().is_gcode());
    }

    match prior {
        // SAFETY: no other test in this binary touches this env var.
        Some(value) => unsafe { std::env::set_var(ORCA_SLICER_EXECUTABLE_ENV, value) },
        None => unsafe { std::env::remove_var(ORCA_SLICER_EXECUTABLE_ENV) },
    }
}
