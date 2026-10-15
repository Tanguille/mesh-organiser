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

    // Relative paths resolve against the crate root, where cargo runs tests. An
    // absolute Unix path would be split off as a `/flag` by the command parser.
    set_custom_slicer_path(&app_state, "missing-slicer --single-instance");
    assert_eq!(
        open_without_paths(&Slicer::Custom, &app_state),
        "Custom slicer executable not found: missing-slicer"
    );

    // Cargo.toml exists but is not executable, so a missed empty-paths check
    // fails to spawn instead of launching something.
    for configured in ["Cargo.toml", "\"Cargo.toml\" --single-instance"] {
        set_custom_slicer_path(&app_state, configured);
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
