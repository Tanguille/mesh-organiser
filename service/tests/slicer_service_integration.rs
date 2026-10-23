//! Characterization tests for `Slicer::open` with no model paths: it must fail
//! before launching anything, and which check fails first decides the message.

mod common;

use service::{AppState, ServiceError, slicer_service::Slicer};

/// Opens `slicer` with no paths; `Ok` would mean a process was spawned.
fn open_without_paths(slicer: &Slicer, app_state: &AppState) -> String {
    match slicer.open(Vec::new(), app_state) {
        Err(ServiceError::InternalError(message)) => message,
        Err(e) => panic!("expected InternalError, got {e:?}"),
        Ok(()) => panic!("opening with no paths must not launch anything"),
    }
}

fn set_custom_slicer_path(app_state: &AppState, path: &str) {
    app_state.configuration.lock().unwrap().custom_slicer_path = path.to_string();
}

#[tokio::test]
async fn custom_slicer_checks_configuration_before_empty_paths() {
    let (_dir, app_state) = common::test_app_state().await;

    set_custom_slicer_path(&app_state, "");
    assert_eq!(
        open_without_paths(&Slicer::Custom, &app_state),
        "Custom slicer path not set"
    );

    // An absolute Unix path would be split off as a `/flag` by the command parser.
    let current_dir = std::env::current_dir().unwrap();
    let slicer_dir = tempfile::Builder::new()
        .prefix("slicer-test-")
        .tempdir_in(&current_dir)
        .unwrap();
    let slicer_path = format!(
        "./{}/slicer",
        slicer_dir
            .path()
            .strip_prefix(&current_dir)
            .unwrap()
            .display()
    );

    set_custom_slicer_path(&app_state, &format!("{slicer_path} --single-instance"));
    assert_eq!(
        open_without_paths(&Slicer::Custom, &app_state),
        format!("Custom slicer executable not found: {slicer_path}")
    );

    // A plain file makes a missed empty-paths check fail instead of launching anything.
    std::fs::write(slicer_dir.path().join("slicer"), []).unwrap();
    for configured in [
        slicer_path.clone(),
        format!("\"{slicer_path}\" --single-instance"),
    ] {
        set_custom_slicer_path(&app_state, &configured);
        assert_eq!(
            open_without_paths(&Slicer::Custom, &app_state),
            "No models to open",
            "custom slicer path {configured:?}"
        );
    }
}

#[tokio::test]
async fn builtin_slicer_checks_installation_before_empty_paths() {
    let (_dir, app_state) = common::test_app_state().await;

    for slicer in [
        Slicer::PrusaSlicer,
        Slicer::OrcaSlicer,
        Slicer::Cura,
        Slicer::BambuStudio,
    ] {
        let expected = if slicer.is_installed() {
            "No models to open"
        } else {
            "Slicer not installed"
        };

        assert_eq!(open_without_paths(&slicer, &app_state), expected);
    }
}
