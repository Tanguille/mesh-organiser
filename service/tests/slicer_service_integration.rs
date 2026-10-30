//! Characterization tests for `Slicer::open`: validation order decides the error,
//! and Linux launch resolves the installed Flatpak package without probing twice.

use std::{env, fs, process::Command};

#[cfg(target_os = "linux")]
use std::time::Duration;

use service::{AppState, ServiceError, slicer_service::Slicer};

mod common;

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

fn assert_child_test_passed(output: &std::process::Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success()
            && stdout.contains("test result: ok. 1 passed; 0 failed; 0 ignored;"),
        "{stdout}\n{stderr}"
    );
}

#[tokio::test]
async fn custom_slicer_checks_configuration_before_empty_paths() {
    if env::var_os("MESH_TEST_CUSTOM_SLICER_CHILD").is_none() {
        let dir = tempfile::tempdir().unwrap();
        let output = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "custom_slicer_checks_configuration_before_empty_paths",
                "--nocapture",
            ])
            .current_dir(dir.path())
            .env("MESH_TEST_CUSTOM_SLICER_CHILD", "1")
            .output()
            .unwrap();
        assert_child_test_passed(&output);

        return;
    }

    let (_dir, app_state) = common::test_app_state().await;

    set_custom_slicer_path(&app_state, "");
    assert_eq!(
        open_without_paths(&Slicer::Custom, &app_state),
        "Custom slicer path not set"
    );

    // An absolute Unix path would be split off as a `/flag` by the command parser.
    let slicer_path = "./slicer";

    set_custom_slicer_path(&app_state, &format!("{slicer_path} --single-instance"));
    assert_eq!(
        open_without_paths(&Slicer::Custom, &app_state),
        format!("Custom slicer executable not found: {slicer_path}")
    );

    // A plain file makes a missed empty-paths check fail instead of launching anything.
    fs::write(slicer_path, []).unwrap();
    for configured in [slicer_path, "\"./slicer\" --single-instance"] {
        set_custom_slicer_path(&app_state, configured);
        assert_eq!(
            open_without_paths(&Slicer::Custom, &app_state),
            "No models to open",
            "custom slicer path {configured:?}"
        );
    }
}

#[tokio::test]
async fn builtin_slicer_empty_paths_reports_host_installation_status() {
    let (_dir, app_state) = common::test_app_state().await;

    // This is a host-dependent smoke check; the Linux shim tests below pin validation order.
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

#[cfg(target_os = "linux")]
fn assert_flatpak_commands(case: &str, installed_package: &str, expected: &[&str]) {
    let dir = tempfile::tempdir().unwrap();
    let shim = dir.path().join("flatpak");
    let log = dir.path().join("commands");
    fs::write(
        &shim,
        "#!/bin/sh\nprintf '%s|' \"$@\" >> \"$MESH_TEST_FLATPAK_LOG\"\nprintf '\\n' >> \"$MESH_TEST_FLATPAK_LOG\"\ncase \"$1\" in\n  info) [ \"$2\" = \"$MESH_TEST_FLATPAK_INSTALLED\" ] ;;\n  run) : > \"$MESH_TEST_FLATPAK_READY\" ;;\n  *) exit 1 ;;\nesac\n",
    )
    .unwrap();
    fs::set_permissions(
        &shim,
        <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
    )
    .unwrap();
    // The shim appends, so an empty log means it never ran (instead of a missing file).
    fs::write(&log, "").unwrap();

    // PATH is changed only for this child, so parallel tests cannot launch a real slicer.
    let output = Command::new(env::current_exe().unwrap())
        .args(["--exact", "linux_flatpak_child", "--ignored", "--nocapture"])
        .env("PATH", dir.path())
        .env("MESH_TEST_FLATPAK_CASE", case)
        .env("MESH_TEST_FLATPAK_INSTALLED", installed_package)
        .env("MESH_TEST_FLATPAK_LOG", &log)
        .env("MESH_TEST_FLATPAK_READY", dir.path().join("ready"))
        .output()
        .unwrap();
    assert_child_test_passed(&output);
    let commands = fs::read_to_string(log).unwrap();

    assert_eq!(commands.lines().collect::<Vec<_>>(), expected, "{case}");
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "invoked by the Flatpak parent tests with an isolated PATH"]
async fn linux_flatpak_child() {
    let case = env::var("MESH_TEST_FLATPAK_CASE")
        .expect("Flatpak child requires the parent test's isolated environment");
    if case == "deserialize" {
        for json in ["{}", r#"{ "data_path": "/custom/data" }"#] {
            let configuration: service::Configuration = serde_json::from_str(json).unwrap();
            assert!(configuration.slicer.is_none(), "{json}");
        }

        return;
    }
    let (_dir, app_state) = common::test_app_state().await;
    let paths = if case.ends_with("empty") {
        Vec::new()
    } else {
        vec![std::path::PathBuf::from("/tmp/model with spaces.stl")]
    };
    let result = Slicer::OrcaSlicer.open(paths, &app_state);
    if case.starts_with("missing") || case == "empty" {
        let expected = if case.starts_with("missing") {
            "Slicer not installed"
        } else {
            "No models to open"
        };
        assert!(matches!(result, Err(ServiceError::InternalError(message)) if message == expected));

        return;
    }
    result.unwrap();

    // spawn() returns before the shim logs its arguments; wait for its completion marker.
    let ready = env::var("MESH_TEST_FLATPAK_READY").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !std::path::Path::new(&ready).exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "Flatpak shim did not run"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_launch_probes_current_flatpak_package_once() {
    assert_flatpak_commands(
        "current",
        "com.orcaslicer.OrcaSlicer",
        &[
            "info|com.orcaslicer.OrcaSlicer|",
            "run|--file-forwarding|com.orcaslicer.OrcaSlicer|@@|/tmp/model with spaces.stl|@@|",
        ],
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_launch_uses_legacy_flatpak_package_without_probing_twice() {
    assert_flatpak_commands(
        "legacy",
        "io.github.softfever.OrcaSlicer",
        &[
            "info|com.orcaslicer.OrcaSlicer|",
            "info|io.github.softfever.OrcaSlicer|",
            "run|--file-forwarding|io.github.softfever.OrcaSlicer|@@|/tmp/model with spaces.stl|@@|",
        ],
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_missing_flatpak_package_never_launches() {
    assert_flatpak_commands(
        "missing",
        "",
        &[
            "info|com.orcaslicer.OrcaSlicer|",
            "info|io.github.softfever.OrcaSlicer|",
        ],
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_missing_flatpak_package_is_reported_before_empty_paths() {
    assert_flatpak_commands(
        "missing-empty",
        "",
        &[
            "info|com.orcaslicer.OrcaSlicer|",
            "info|io.github.softfever.OrcaSlicer|",
        ],
    );
}

/// Every settings save deserializes a `Configuration`, and its container-level
/// `#[serde(default)]` builds `Configuration::default()` each time, so `Default` must not
/// probe Flatpak for each slicer. Deserializing spawns nothing, and a missing slicer key
/// stays `None` even when a slicer is installed.
#[cfg(target_os = "linux")]
#[test]
fn linux_configuration_deserialize_never_probes_flatpak() {
    assert_flatpak_commands("deserialize", "com.prusa3d.PrusaSlicer", &[]);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_installed_flatpak_package_with_empty_paths_never_launches() {
    assert_flatpak_commands(
        "empty",
        "com.orcaslicer.OrcaSlicer",
        &["info|com.orcaslicer.OrcaSlicer|"],
    );
}
