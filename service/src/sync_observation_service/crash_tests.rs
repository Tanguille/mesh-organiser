//! Crash checkpoints run in child processes; no production environment hook exists.
use std::{
    os::unix::fs::MetadataExt,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};

use super::*;

static RESTORE_SYNC_ARMED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn cleaned_stage(path: &Path) -> bool {
    !path.exists()
        || (path.is_dir()
            && std::fs::read_dir(path).is_ok_and(|entries| {
                entries.into_iter().all(|entry| {
                    entry.is_ok_and(|entry| {
                        entry.file_name() == ".lease"
                            && entry.file_type().is_ok_and(|kind| kind.is_file())
                    })
                })
            }))
}

fn no_payload_stages(root: &Path) -> bool {
    std::fs::read_dir(root).unwrap().all(|entry| {
        let path = entry.unwrap().path();
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".sync-stage-")
            && cleaned_stage(&path)
    })
}

pub(super) fn directory_synced(path: &Path) -> Result<(), ServiceError> {
    if !RESTORE_SYNC_ARMED.load(Ordering::SeqCst) {
        return Ok(());
    }
    let root = std::env::var("MESH_RECOVERY_TEST_ROOT")
        .or_else(|_| std::env::var("MESH_GC_TEST_ROOT"))
        .unwrap();
    if std::fs::canonicalize(path)? == std::fs::canonicalize(Path::new(&root).join("models"))? {
        let marker = std::env::var("MESH_RESTORE_SYNC_MARKER").unwrap();
        std::fs::write(marker, b"synced")?;
        RESTORE_SYNC_ARMED.store(false, Ordering::SeqCst);
    }

    Ok(())
}

pub(super) fn checkpoint(point: &str) -> Result<(), ServiceError> {
    if let Ok(marker) = std::env::var("MESH_RESTORE_SYNC_MARKER") {
        if point == "before_restore_directory_sync"
            && std::env::var_os("MESH_RESTORE_FAIL").is_some()
        {
            std::fs::write(&marker, b"attempted")?;

            return Err(std::io::Error::other("injected restore directory sync failure").into());
        }
        if point == "before_restore_directory_sync" {
            RESTORE_SYNC_ARMED.store(true, Ordering::SeqCst);
        }
        if point == "before_restore_unlink" && !Path::new(&marker).exists() {
            return Err(std::io::Error::other("restore unlink preceded directory sync").into());
        }
    }
    let Ok(wanted) = std::env::var("MESH_TRANSFER_TEST_POINT") else {
        return Ok(());
    };
    if wanted != point {
        return Ok(());
    }
    let marker = std::env::var("MESH_TRANSFER_TEST_MARKER").unwrap();
    std::fs::write(marker, point)?;
    if point == "outbox_before_retention_check" {
        return Ok(());
    }
    if point == "before_stage_directory_remove" {
        let root = Path::new(&std::env::var("MESH_GC_TEST_ROOT").unwrap()).join("models");
        let stage = std::fs::read_dir(root)?
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".sync-stage-")
                    && path.join(".lease").is_file()
            })
            .unwrap();
        std::fs::rename(&stage, std::env::var("MESH_GC_SWAP_STAGE").unwrap())?;
        std::fs::create_dir(&stage)?;
        std::fs::write(
            std::env::var("MESH_GC_REPLACEMENT_PATH").unwrap(),
            stage.to_string_lossy().as_bytes(),
        )?;

        return Ok(());
    }
    if point == "before_orphan_capture" {
        if std::env::var_os("MESH_GC_COLLIDE_QUARANTINE").is_some() {
            let root = Path::new(&std::env::var("MESH_GC_TEST_ROOT").unwrap()).join("models");
            let stage = std::fs::read_dir(root)?
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".sync-stage-")
                        && path.join(".lease").is_file()
                })
                .unwrap();
            std::fs::write(stage.join(".cleanup-0"), b"unrelated sentinel")?;
            std::fs::write(
                std::env::var("MESH_GC_REPLACEMENT_PATH").unwrap(),
                stage.to_string_lossy().as_bytes(),
            )?;

            return Ok(());
        }
        if let Ok(parked) = std::env::var("MESH_GC_SWAP_STAGE") {
            let root = Path::new(&std::env::var("MESH_GC_TEST_ROOT").unwrap()).join("models");
            let stage = std::fs::read_dir(root)?
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".sync-stage-")
                        && path.join(".lease").is_file()
                })
                .unwrap();
            std::fs::rename(&stage, parked)?;
            std::fs::create_dir(&stage)?;
            std::fs::write(stage.join(".cleanup-0"), b"unrelated sentinel")?;
            std::fs::write(
                std::env::var("MESH_GC_REPLACEMENT_PATH").unwrap(),
                stage.to_string_lossy().as_bytes(),
            )?;

            return Ok(());
        }
        std::fs::rename(
            std::env::var("MESH_GC_REPLACEMENT").unwrap(),
            std::env::var("MESH_GC_OWNED_PATH").unwrap(),
        )?;

        return Ok(());
    }
    if point == "before_final_directory_sync" {
        return Err(std::io::Error::other("injected directory sync failure").into());
    }
    loop {
        std::thread::park_timeout(Duration::from_secs(1));
    }
}

async fn app(root: &Path) -> AppState {
    std::fs::create_dir_all(root.join("backups")).unwrap();
    let db = db::db_context::setup_db(&root.join("db.sqlite"), &root.join("backups")).await;
    let data_path = root.to_string_lossy().into_owned();
    AppState {
        db: Arc::new(db),
        configuration: Mutex::new(crate::Configuration {
            data_path: data_path.clone(),
            ..Default::default()
        }),
        import_mutex: Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: data_path,
    }
}

#[tokio::test]
async fn sequential_finished_stages_reuse_one_certified_slot() {
    let dir = tempfile::tempdir().unwrap();
    let state = app(dir.path()).await;
    let mut first = Stage::new(&state).await.unwrap();
    let path = first.path().to_path_buf();
    let meta = std::fs::metadata(&path).unwrap();
    let original = (meta.dev(), meta.ino());
    first.finish(&state).await.unwrap();
    drop(first);
    let reopened = app(dir.path()).await;
    let second = Stage::new(&reopened).await.unwrap();
    let meta = std::fs::metadata(second.path()).unwrap();
    assert_eq!(
        second.path(),
        path,
        "a finished certified slot must be reused"
    );
    assert_eq!(
        (meta.dev(), meta.ino()),
        original,
        "reuse must keep the certified directory identity"
    );
    assert!(path.join(".lease").is_file());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transfer_intents")
            .fetch_one(&*state.db)
            .await
            .unwrap(),
        1,
        "only the currently active stage has an intent"
    );
}

#[tokio::test]
async fn replacing_open_database_path_cannot_certify_or_reuse_a_stage() {
    let dir = tempfile::tempdir().unwrap();
    let state = app(dir.path()).await;
    let mut first = Stage::new(&state).await.unwrap();
    let stage_path = first.path().to_path_buf();
    first.finish(&state).await.unwrap();
    drop(first);

    let db_path = dir.path().join("db.sqlite");
    let copy_path = dir.path().join("copy.sqlite");
    sqlx::query("VACUUM INTO ?")
        .bind(copy_path.to_string_lossy().into_owned())
        .execute(&*state.db)
        .await
        .unwrap();
    let parked_path = dir.path().join("parked.sqlite");
    std::fs::rename(&db_path, &parked_path).unwrap();
    std::fs::rename(&copy_path, &db_path).unwrap();
    let result = Stage::new(&state).await;
    std::fs::rename(&db_path, &copy_path).unwrap();
    std::fs::rename(&parked_path, &db_path).unwrap();

    assert!(
        result.is_err(),
        "a replaced path must not certify the open database"
    );
    assert!(stage_path.join(".lease").is_file());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_stage_slots")
            .fetch_one(&*state.db)
            .await
            .unwrap(),
        1,
        "a failed ownership check must not reserve another slot"
    );
}

#[tokio::test]
async fn unrelated_child_process_does_not_inherit_and_hold_a_stage_lease() {
    let dir = tempfile::tempdir().unwrap();
    let context = app(dir.path()).await;
    let mut stage = Stage::new(&context).await.unwrap();
    let path = stage.path().to_path_buf();
    let mut child = KillOnDrop(
        Command::new("/bin/sleep")
            .arg("10")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    stage.finish(&context).await.unwrap();
    drop(stage);
    let next = Stage::new(&context).await.unwrap();
    assert_eq!(
        next.path(),
        path,
        "an unrelated exec child must not keep an idle slot locked"
    );
    child.0.kill().unwrap();
    child.0.wait().unwrap();
}

#[tokio::test]
async fn idle_slot_reuse_does_not_deadlock_a_single_connection_pool() {
    let dir = tempfile::tempdir().unwrap();
    let mut context = app(dir.path()).await;
    let mut stage = Stage::new(&context).await.unwrap();
    let path = stage.path().to_path_buf();
    stage.finish(&context).await.unwrap();
    drop(stage);
    context.db = Arc::new(
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("db.sqlite")),
            )
            .await
            .unwrap(),
    );
    let reused = tokio::time::timeout(Duration::from_secs(2), Stage::new(&context))
        .await
        .expect("slot reuse must not wait for a second database connection")
        .unwrap();
    assert_eq!(reused.path(), path);
}

#[tokio::test]
async fn live_and_pending_outbox_stages_are_never_reused() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("queued");
    let mut process = outbox_child(dir.path(), "outbox_committed", &marker);
    wait_marker(&mut process, &marker).await;
    let source = app(&dir.path().join("source")).await;
    let pending: String = sqlx::query_scalar("SELECT stage_path FROM sync_publication_stages")
        .fetch_one(&*source.db)
        .await
        .unwrap();
    let mut separate = Stage::new(&source).await.unwrap();
    let separate_path = separate.path().to_path_buf();
    assert_ne!(
        separate_path,
        Path::new(&pending),
        "a stage leased by another process cannot be reused"
    );
    separate.finish(&source).await.unwrap();
    drop(separate);
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    recover_transfers(&source).await.unwrap();
    let idle:Vec<String> = sqlx::query_scalar("SELECT stage_path FROM sync_stage_slots WHERE directory_inode IS NOT NULL AND stage_path<>?").bind(&pending).fetch_all(&*source.db).await.unwrap();
    let prior_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_stage_slots")
        .fetch_one(&*source.db)
        .await
        .unwrap();
    let stage = Stage::new(&source).await.unwrap();
    assert_ne!(
        stage.path(),
        Path::new(&pending),
        "a pending outbox stage stays reserved after its author exits"
    );
    assert!(
        idle.iter().any(|path| Path::new(path) == stage.path()),
        "an existing idle certified slot must be reused; idle={idle:?}, actual={:?}",
        stage.path()
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_stage_slots")
        .fetch_one(&*source.db)
        .await
        .unwrap();
    assert_eq!(count, prior_count, "reuse cannot allocate another slot");
}

#[tokio::test]
async fn crash_before_slot_certification_never_adopts_its_reserved_name() {
    for point in ["stage_slot_reserved", "stage_slot_created"] {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("slot-checkpoint");
        let mut process = outbox_child(dir.path(), point, &marker);
        wait_marker(&mut process, &marker).await;
        process.0.kill().unwrap();
        process.0.wait().unwrap();
        let source = app(&dir.path().join("source")).await;
        let (old, cert): (String, Option<String>) =
            sqlx::query_as("SELECT stage_path,directory_inode FROM sync_stage_slots")
                .fetch_one(&*source.db)
                .await
                .unwrap();
        assert_eq!(
            cert, None,
            "an interrupted reservation must not become certified"
        );
        let stage = Stage::new(&source).await.unwrap();
        assert_ne!(
            stage.path(),
            Path::new(&old),
            "an uncertified name cannot be adopted"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_stage_slots")
                .fetch_one(&*source.db)
                .await
                .unwrap(),
            2
        );
    }
}

#[tokio::test]
async fn idle_slot_with_replaced_directory_or_lease_is_not_adopted() {
    for replace_directory in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let context = app(dir.path()).await;
        let mut stage = Stage::new(&context).await.unwrap();
        let path = stage.path().to_path_buf();
        stage.finish(&context).await.unwrap();
        drop(stage);
        let saved = dir.path().join("saved");
        if replace_directory {
            std::fs::rename(&path, &saved).unwrap();
            std::fs::create_dir(&path).unwrap();
            std::fs::copy(saved.join(".lease"), path.join(".lease")).unwrap();
        } else {
            std::fs::rename(path.join(".lease"), &saved).unwrap();
            std::fs::copy(&saved, path.join(".lease")).unwrap();
        }
        std::fs::write(path.join("unrelated"), b"keep").unwrap();
        if let Ok(other) = Stage::new(&context).await {
            assert_ne!(
                other.path(),
                path,
                "replaced directory or lease cannot be adopted"
            );
        }
        assert_eq!(std::fs::read(path.join("unrelated")).unwrap(), b"keep");
    }
}

#[tokio::test]
async fn copied_database_cannot_reuse_another_database_file_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let state = app(dir.path()).await;
    let mut first = Stage::new(&state).await.unwrap();
    let original = first.path().to_path_buf();
    first.finish(&state).await.unwrap();
    drop(first);
    let clone_path = dir.path().join("clone.sqlite");
    sqlx::query("VACUUM INTO ?")
        .bind(clone_path.to_string_lossy().into_owned())
        .execute(&*state.db)
        .await
        .unwrap();
    let clone_db = db::db_context::setup_db(&clone_path, &dir.path().join("backups")).await;
    let data_path = dir.path().to_string_lossy().into_owned();
    let clone = AppState {
        db: Arc::new(clone_db),
        configuration: Mutex::new(crate::Configuration {
            data_path: data_path.clone(),
            ..Default::default()
        }),
        import_mutex: Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: data_path,
    };
    if let Ok(stage) = Stage::new(&clone).await {
        assert_ne!(
            stage.path(),
            original,
            "a copied DB cannot authorize another file's stage certificate"
        );
    }
    assert!(original.join(".lease").is_file());
}

#[tokio::test]
async fn exhausted_slot_reservations_reject_before_creating_more_paths() {
    let dir = tempfile::tempdir().unwrap();
    let state = app(dir.path()).await;
    let root = state.get_model_dir();
    let database = dir.path().join("db.sqlite");
    let mut tx = state.db.begin().await.unwrap();
    for index in 0..db::sync_transport::stage_slots::MAX_STAGE_SLOTS {
        let path = root.join(format!(".sync-stage-reserved-{index}"));
        sqlx::query("INSERT INTO sync_stage_slots(stage_path,root_path,root_device,root_inode,database_path,database_device,database_inode) VALUES(?,?,?,?,?,?,?)")
            .bind(path.to_string_lossy().into_owned()).bind(root.to_string_lossy().into_owned()).bind("1").bind("1").bind(database.to_string_lossy().into_owned()).bind("1").bind("1").execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    assert!(
        Stage::new(&state).await.is_err(),
        "uncertified reservations count toward the hard capacity"
    );
    assert_eq!(
        std::fs::read_dir(root).unwrap().count(),
        0,
        "exhausted capacity must reject before mkdir"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines, clippy::significant_drop_tightening)] // The live fixtures own stages until the checkpoint.
async fn crash_child() {
    let Ok(root) = std::env::var("MESH_TRANSFER_TEST_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let source = app(&root.join("source")).await;
    let hub = app(&root.join("hub")).await;
    let user = User::default();
    let source_identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let hub_identity = db::user_db::get_sync_identity(&hub.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let registration = db::sync_transport::register_writer(
        &hub.db,
        &user,
        &hub_identity.account_id,
        &source_identity,
    )
    .await
    .unwrap();
    db::sync_peer::bind_peer(
        &source.db,
        &user,
        &source_identity,
        &registration.server_identity,
    )
    .await
    .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &source_identity, &registration)
        .await
        .unwrap();
    let global = db::random_hex_32();
    let group = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('crash','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(global).execute(&*source.db).await.unwrap().last_insert_rowid();
    let source_path = root.join("source.stl");
    std::fs::write(&source_path, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &source.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        Some(source_path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let model = db::model_db::add_model(&source.db, &user, "part", blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&source.db, &user, Some(group), vec![model], None)
        .await
        .unwrap();
    let prepared = prepare_observed_group(&source, &user, &source_identity, group)
        .await
        .unwrap();
    let proof = db::sync_transport::writer_proof(
        &source.db,
        &user,
        &source_identity,
        prepared.group_event(),
    )
    .await
    .unwrap();
    let plan = observations::inspect(
        &hub.db,
        &user,
        &hub_identity.account_id,
        &registration.connection_id,
        &registration.writer_secret,
        Some(&proof.writer_nonce),
        prepared.item().clone(),
    )
    .await
    .unwrap();
    let mut incoming = IncomingObservation::new(&hub, &plan).await.unwrap();
    let mut blob = incoming
        .start_blob(prepared.blobs()[0].key())
        .await
        .unwrap();
    blob.write(b"abc").await.unwrap();
    incoming.finish_blob(blob).await.unwrap();
    let result = install_received(
        hub,
        user,
        hub_identity.account_id,
        registration.connection_id,
        registration.writer_secret,
        Some(proof.writer_nonce),
        prepared.item().clone(),
        incoming,
    )
    .await;
    if std::env::var("MESH_TRANSFER_TEST_POINT").unwrap() == "before_final_directory_sync" {
        assert!(
            result.is_err(),
            "directory sync failure must prevent a commit"
        );
    } else {
        panic!("requested crash checkpoint was not reached: {result:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn outbox_crash_child() {
    let Ok(root) = std::env::var("MESH_OUTBOX_TEST_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let source = app(&root.join("source")).await;
    let hub = app(&root.join("hub")).await;
    let user = User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let hub_identity = db::user_db::get_sync_identity(&hub.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let registration =
        db::sync_transport::register_writer(&hub.db, &user, &hub_identity.account_id, &identity)
            .await
            .unwrap();
    let origin = "https://outbox.test";
    let token = db::random_hex_32();
    db::user_db::set_user_sync_token(&source.db, user.id, &token, origin, false)
        .await
        .unwrap();
    db::sync_peer::bind_peer(&source.db, &user, &identity, &registration.server_identity)
        .await
        .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &registration)
        .await
        .unwrap();
    let authorization = db::sync_transport::pull::Authorization::capture(
        &source.db, &user, &identity, origin, &token,
    )
    .await
    .unwrap();
    let group = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('outbox','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(db::random_hex_32()).execute(&*source.db).await.unwrap().last_insert_rowid();
    let path = root.join("source.stl");
    std::fs::write(&path, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &source.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        Some(path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let model = db::model_db::add_model(&source.db, &user, "part", blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&source.db, &user, Some(group), vec![model], None)
        .await
        .unwrap();
    let result = queue_observed_group(&source, &user, &identity, &authorization, group).await;
    panic!("requested outbox crash checkpoint not reached: {result:?}");
}

fn outbox_child(root: &Path, point: &str, marker: &Path) -> KillOnDrop {
    KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sync_observation_service::crash_tests::outbox_crash_child",
                "--nocapture",
            ])
            .env("MESH_OUTBOX_TEST_ROOT", root)
            .env("MESH_TRANSFER_TEST_POINT", point)
            .env("MESH_TRANSFER_TEST_MARKER", marker)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

#[tokio::test]
async fn killed_queue_recovers_only_uncommitted_certified_stages() {
    for (point, committed) in [("outbox_files_synced", false), ("outbox_committed", true)] {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("checkpoint");
        let mut process = outbox_child(dir.path(), point, &marker);
        wait_marker(&mut process, &marker).await;
        let source = app(&dir.path().join("source")).await;
        let model_dir = source.get_model_dir();
        let unrelated = model_dir.join("unrelated.stl");
        std::fs::write(&unrelated, b"keep").unwrap();
        recover_transfers(&source).await.unwrap();
        assert!(
            std::fs::read_dir(&model_dir).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".sync-stage-")),
            "live process stage must remain leased"
        );
        process.0.kill().unwrap();
        process.0.wait().unwrap();
        recover_transfers(&source).await.unwrap();
        let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_publication_outbox")
            .fetch_one(&*source.db)
            .await
            .unwrap();
        let stage_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_publication_stages")
            .fetch_one(&*source.db)
            .await
            .unwrap();
        assert_eq!(
            (queued, stage_count),
            (i64::from(committed), i64::from(committed))
        );
        let stages: Vec<_> = std::fs::read_dir(&model_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".sync-stage-")
                    && !cleaned_stage(path)
            })
            .collect();
        assert_eq!(
            stages.len(),
            usize::from(committed),
            "only committed pending stages survive recovery"
        );
        assert_eq!(std::fs::read(unrelated).unwrap(), b"keep");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn outbox_recovery_race_child() {
    let Ok(root) = std::env::var("MESH_OUTBOX_TEST_ROOT") else {
        return;
    };
    let ready = std::env::var("MESH_OUTBOX_RACE_READY").unwrap();
    let go = std::env::var("MESH_OUTBOX_RACE_GO").unwrap();
    let source = app(&Path::new(&root).join("source")).await;
    std::fs::write(ready, b"ready").unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !Path::new(&go).exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    recover_transfers(&source).await.unwrap();
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Recovery must serialize its retention decision with a concurrent outbox writer.
async fn recovery_cannot_delete_a_stage_with_an_inflight_outbox_reference() {
    let dir = tempfile::tempdir().unwrap();
    let queued_marker = dir.path().join("queued");
    let mut queued = outbox_child(dir.path(), "outbox_committed", &queued_marker);
    wait_marker(&mut queued, &queued_marker).await;
    queued.0.kill().unwrap();
    queued.0.wait().unwrap();
    let source = app(&dir.path().join("source")).await;
    let (id, path, token, files, bytes): (String, String, String, String, i64) = sqlx::query_as(
        "SELECT outbox_id,stage_path,token,files,logical_bytes FROM sync_publication_stages",
    )
    .fetch_one(&*source.db)
    .await
    .unwrap();
    sqlx::query("DELETE FROM sync_publication_stages WHERE outbox_id=?")
        .bind(&id)
        .execute(&*source.db)
        .await
        .unwrap();
    let ready = dir.path().join("recovery-ready");
    let go = dir.path().join("recovery-go");
    let checked = dir.path().join("retention-check");
    let mut recovery = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sync_observation_service::crash_tests::outbox_recovery_race_child",
                "--nocapture",
            ])
            .env("MESH_OUTBOX_TEST_ROOT", dir.path())
            .env("MESH_OUTBOX_RACE_READY", &ready)
            .env("MESH_OUTBOX_RACE_GO", &go)
            .env("MESH_TRANSFER_TEST_POINT", "outbox_before_retention_check")
            .env("MESH_TRANSFER_TEST_MARKER", &checked)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_marker(&mut recovery, &ready).await;
    let mut tx = source.db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("INSERT INTO sync_publication_stages(outbox_id,stage_path,token,files,logical_bytes) VALUES(?,?,?,?,?)").bind(&id).bind(&path).bind(&token).bind(&files).bind(bytes).execute(&mut *tx).await.unwrap();
    std::fs::write(go, b"go").unwrap();
    wait_marker(&mut recovery, &checked).await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let remained = Path::new(&path).exists();
    tx.commit().await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(status) = recovery.0.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("recovery must finish after writer commits");
    assert!(
        remained && Path::new(&path).exists(),
        "recovery must not remove a stage while its reference is committing"
    );
    assert!(status.success(), "recovery process failed");
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_publication_stages WHERE outbox_id=?")
            .bind(&id)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn deleting_queue_owner_allows_only_its_certified_stage_to_be_cleaned() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("queued");
    let mut process = outbox_child(dir.path(), "outbox_committed", &marker);
    wait_marker(&mut process, &marker).await;
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    let source = app(&dir.path().join("source")).await;
    let path: String = sqlx::query_scalar("SELECT stage_path FROM sync_publication_stages")
        .fetch_one(&*source.db)
        .await
        .unwrap();
    let unrelated = source.get_model_dir().join("keep.stl");
    std::fs::write(&unrelated, b"keep").unwrap();
    sqlx::query("DELETE FROM sync_accounts WHERE user_id=?")
        .bind(User::default().id)
        .execute(&*source.db)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_publication_stages")
            .fetch_one(&*source.db)
            .await
            .unwrap(),
        0
    );
    recover_transfers(&source).await.unwrap();
    assert!(cleaned_stage(Path::new(&path)));
    assert_eq!(std::fs::read(unrelated).unwrap(), b"keep");
}

struct KillOnDrop(std::process::Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn child(root: &Path, point: &str, marker: &Path) -> KillOnDrop {
    KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sync_observation_service::crash_tests::crash_child",
                "--nocapture",
            ])
            .env("MESH_TRANSFER_TEST_ROOT", root)
            .env("MESH_TRANSFER_TEST_POINT", point)
            .env("MESH_TRANSFER_TEST_MARKER", marker)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

async fn wait_marker(child: &mut KillOnDrop, marker: &Path) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !marker.exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "child exited before crash checkpoint"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("child did not reach crash checkpoint");
}

#[tokio::test]
async fn killed_transfers_recover_only_certified_uncommitted_files() {
    for point in ["stage_written", "files_published", "receipt_committed"] {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("checkpoint");
        let mut process = child(dir.path(), point, &marker);
        wait_marker(&mut process, &marker).await;
        process.0.kill().unwrap();
        process.0.wait().unwrap();
        let hub = app(&dir.path().join("hub")).await;
        let untouched = hub.get_model_dir().join(".sync-stage-unrelated");
        std::fs::create_dir(&untouched).unwrap();
        std::fs::write(untouched.join("keep"), b"keep").unwrap();
        recover_transfers(&hub).await.unwrap();
        let intents: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_transfer_intents")
            .fetch_one(&*hub.db)
            .await
            .unwrap();
        assert_eq!(
            intents, 0,
            "recovery must clear completed intent at {point}"
        );
        let files: Vec<_> = std::fs::read_dir(hub.get_model_dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            std::fs::read(untouched.join("keep")).unwrap(),
            b"keep",
            "unproven paths must never be removed"
        );
        assert_eq!(
            std::fs::read(dir.path().join("source.stl")).unwrap(),
            b"abc",
            "recovery must not remove external source files"
        );
        assert!(
            !files.iter().any(|name| name.starts_with(".sync-stage-")
                && name != ".sync-stage-unrelated"
                && !cleaned_stage(&hub.get_model_dir().join(name))),
            "crashed stage must be recovered at {point}: {files:?}"
        );
        let logs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_transport_log")
            .fetch_one(&*hub.db)
            .await
            .unwrap();
        let committed = point == "receipt_committed";
        assert_eq!(logs, i64::from(committed));
        let owned: Vec<_> = files
            .iter()
            .filter(|name| name.starts_with(".sync-blob-"))
            .collect();
        assert_eq!(
            owned.len(),
            usize::from(committed),
            "only committed content survives {point}"
        );
        if committed {
            assert_eq!(
                std::fs::read(hub.get_model_dir().join(owned[0])).unwrap(),
                b"abc"
            );
        }
    }
}

#[tokio::test]
async fn recovery_does_not_reap_a_live_other_process() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("checkpoint");
    let mut process = child(dir.path(), "stage_written", &marker);
    wait_marker(&mut process, &marker).await;
    let hub = app(&dir.path().join("hub")).await;
    let before = std::fs::read_dir(hub.get_model_dir()).unwrap().count();
    assert!(before > 0, "the live child must own a staged transfer");
    recover_transfers(&hub).await.unwrap();
    let live_intents: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_transfer_intents")
        .fetch_one(&*hub.db)
        .await
        .unwrap();
    assert_eq!(
        live_intents, 1,
        "a live lease must retain its durable intent"
    );
    assert_eq!(
        std::fs::read_dir(hub.get_model_dir()).unwrap().count(),
        before,
        "live process lease must protect its stage"
    );
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    recover_transfers(&hub).await.unwrap();
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_transfer_intents")
        .fetch_one(&*hub.db)
        .await
        .unwrap();
    assert_eq!(remaining, 0, "recovery must clear the dead process intent");
    assert!(
        no_payload_stages(&hub.get_model_dir()),
        "released lease permits recovery"
    );
}

#[tokio::test]
async fn directory_sync_failure_cannot_commit_or_leave_owned_files() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("checkpoint");
    let mut process = child(dir.path(), "before_final_directory_sync", &marker);
    wait_marker(&mut process, &marker).await;
    let status = process.0.wait().unwrap();
    assert!(status.success(), "child must observe the injected error");
    let hub = app(&dir.path().join("hub")).await;
    recover_transfers(&hub).await.unwrap();
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_transfer_intents")
        .fetch_one(&*hub.db)
        .await
        .unwrap();
    assert_eq!(remaining, 0);
    let logs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_transport_log")
        .fetch_one(&*hub.db)
        .await
        .unwrap();
    assert_eq!(logs, 0);
    assert!(no_payload_stages(&hub.get_model_dir()));
}

#[tokio::test]
async fn orphan_gc_preserves_replacement_file_and_identity_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("checkpoint");
    let mut process = child(dir.path(), "receipt_committed", &marker);
    wait_marker(&mut process, &marker).await;
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    let hub = app(&dir.path().join("hub")).await;
    recover_transfers(&hub).await.unwrap();
    let owned: String =
        sqlx::query_scalar("SELECT path FROM sync_owned_blob_files WHERE blob_id IS NOT NULL")
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    assert_eq!(std::fs::read(&owned).unwrap(), b"abc");
    sqlx::query("DELETE FROM models")
        .execute(&*hub.db)
        .await
        .unwrap();
    db::blob_db::get_and_delete_dead_blobs(&hub.db)
        .await
        .unwrap();
    let orphaned: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sync_owned_blob_files WHERE path=? AND blob_id IS NULL",
    )
    .bind(&owned)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(orphaned, 1);
    let replacement = dir.path().join("replacement");
    std::fs::write(&replacement, b"abc").unwrap();
    assert_ne!(
        std::fs::metadata(&replacement).unwrap().ino(),
        std::fs::metadata(&owned).unwrap().ino(),
        "the replacement must be a different inode even when bytes match"
    );
    std::fs::rename(&replacement, &owned).unwrap();
    let _result = export_service::delete_dead_blobs(&hub).await;
    assert!(
        Path::new(&owned).exists(),
        "GC must not remove a replacement file"
    );
    assert_eq!(
        std::fs::read(&owned).unwrap(),
        b"abc",
        "GC must not delete a different file that replaced an owned path"
    );
    let retained: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sync_owned_blob_files WHERE path=? AND blob_id IS NULL",
    )
    .bind(&owned)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(
        retained, 1,
        "mismatched ownership evidence must remain for safe recovery"
    );
}

#[tokio::test]
async fn orphan_gc_race_child() {
    let Ok(root) = std::env::var("MESH_GC_TEST_ROOT") else {
        return;
    };
    let hub = app(Path::new(&root)).await;
    let _result = export_service::delete_dead_blobs(&hub).await;
}

#[tokio::test]
async fn orphan_gc_restores_file_replaced_at_capture_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("commit-checkpoint");
    let mut process = child(dir.path(), "receipt_committed", &marker);
    wait_marker(&mut process, &marker).await;
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    let hub = app(&dir.path().join("hub")).await;
    recover_transfers(&hub).await.unwrap();
    let owned: String =
        sqlx::query_scalar("SELECT path FROM sync_owned_blob_files WHERE blob_id IS NOT NULL")
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    sqlx::query("DELETE FROM models")
        .execute(&*hub.db)
        .await
        .unwrap();
    db::blob_db::get_and_delete_dead_blobs(&hub.db)
        .await
        .unwrap();
    let replacement = dir.path().join("raced-replacement");
    std::fs::write(&replacement, b"raced replacement").unwrap();
    let race_marker = dir.path().join("race-checkpoint");
    let sync_marker = dir.path().join("root-synced");
    let mut process = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sync_observation_service::crash_tests::orphan_gc_race_child",
                "--nocapture",
            ])
            .env("MESH_GC_TEST_ROOT", dir.path().join("hub"))
            .env("MESH_GC_REPLACEMENT", &replacement)
            .env("MESH_GC_OWNED_PATH", &owned)
            .env("MESH_TRANSFER_TEST_POINT", "before_orphan_capture")
            .env("MESH_TRANSFER_TEST_MARKER", &race_marker)
            .env("MESH_RESTORE_SYNC_MARKER", &sync_marker)
            .env("MESH_RESTORE_FAIL", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_marker(&mut process, &race_marker).await;
    assert!(process.0.wait().unwrap().success());
    assert!(
        sync_marker.exists(),
        "recovery must attempt the root sync before releasing quarantine"
    );
    assert!(
        Path::new(&owned).exists(),
        "a raced replacement must not disappear"
    );
    assert_eq!(
        std::fs::read(&owned).unwrap(),
        b"raced replacement",
        "a raced replacement must be restored"
    );
    let retained: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sync_owned_blob_files WHERE path=? AND blob_id IS NULL",
    )
    .bind(&owned)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(retained, 1, "raced ownership evidence must remain");
    let stage: String = sqlx::query_scalar("SELECT stage_path FROM sync_transfer_intents")
        .fetch_one(&*hub.db)
        .await
        .unwrap();
    assert!(
        Path::new(&stage).join(".cleanup-0").exists(),
        "failed sync must retain the quarantined replacement"
    );
    let retry_done = dir.path().join("race-retry-done");
    let retry_sync = dir.path().join("race-retry-synced");
    let mut process = recovery_child(&dir.path().join("hub"), &retry_done, &retry_sync, false);
    wait_marker(&mut process, &retry_done).await;
    assert!(process.0.wait().unwrap().success());
    assert!(
        retry_sync.exists(),
        "retry must sync the restored name before unlinking quarantine"
    );
    assert_eq!(std::fs::read(&owned).unwrap(), b"raced replacement");
    assert!(cleaned_stage(Path::new(&stage)));
}

#[tokio::test]
async fn orphan_capture_never_overwrites_sentinel_after_stage_swap() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("committed");
    let mut process = child(dir.path(), "receipt_committed", &marker);
    wait_marker(&mut process, &marker).await;
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    let hub = app(&dir.path().join("hub")).await;
    recover_transfers(&hub).await.unwrap();
    let owned: String =
        sqlx::query_scalar("SELECT path FROM sync_owned_blob_files WHERE blob_id IS NOT NULL")
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    sqlx::query("DELETE FROM models")
        .execute(&*hub.db)
        .await
        .unwrap();
    db::blob_db::get_and_delete_dead_blobs(&hub.db)
        .await
        .unwrap();
    let external = dir.path().join("external.stl");
    std::fs::write(&external, b"external").unwrap();
    let race = dir.path().join("race");
    let parked = dir.path().join("parked-stage");
    let replacement_path = dir.path().join("replacement-path");
    let mut process = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sync_observation_service::crash_tests::orphan_gc_race_child",
                "--nocapture",
            ])
            .env("MESH_GC_TEST_ROOT", dir.path().join("hub"))
            .env("MESH_GC_SWAP_STAGE", &parked)
            .env("MESH_GC_REPLACEMENT_PATH", &replacement_path)
            .env("MESH_TRANSFER_TEST_POINT", "before_orphan_capture")
            .env("MESH_TRANSFER_TEST_MARKER", &race)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_marker(&mut process, &race).await;
    assert!(process.0.wait().unwrap().success());
    let replacement = std::fs::read_to_string(replacement_path).unwrap();
    assert_eq!(
        std::fs::read(Path::new(&replacement).join(".cleanup-0")).unwrap(),
        b"unrelated sentinel",
        "cleanup must not overwrite or remove a replacement sentinel"
    );
    assert_eq!(
        std::fs::read(&owned).unwrap(),
        b"abc",
        "stage swap must not delete the original owned file"
    );
    assert_eq!(std::fs::read(external).unwrap(), b"external");
}

#[tokio::test]
async fn orphan_capture_never_overwrites_existing_quarantine_entry() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("committed");
    let mut process = child(dir.path(), "receipt_committed", &marker);
    wait_marker(&mut process, &marker).await;
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    let hub = app(&dir.path().join("hub")).await;
    recover_transfers(&hub).await.unwrap();
    let owned: String =
        sqlx::query_scalar("SELECT path FROM sync_owned_blob_files WHERE blob_id IS NOT NULL")
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    sqlx::query("DELETE FROM models")
        .execute(&*hub.db)
        .await
        .unwrap();
    db::blob_db::get_and_delete_dead_blobs(&hub.db)
        .await
        .unwrap();
    let marker = dir.path().join("collision");
    let stage_path = dir.path().join("stage-path");
    let mut process = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sync_observation_service::crash_tests::orphan_gc_race_child",
                "--nocapture",
            ])
            .env("MESH_GC_TEST_ROOT", dir.path().join("hub"))
            .env("MESH_GC_COLLIDE_QUARANTINE", "1")
            .env("MESH_GC_REPLACEMENT_PATH", &stage_path)
            .env("MESH_TRANSFER_TEST_POINT", "before_orphan_capture")
            .env("MESH_TRANSFER_TEST_MARKER", &marker)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_marker(&mut process, &marker).await;
    assert!(process.0.wait().unwrap().success());
    let stage = std::fs::read_to_string(stage_path).unwrap();
    assert_eq!(
        std::fs::read(Path::new(&stage).join(".cleanup-0")).unwrap(),
        b"unrelated sentinel"
    );
    assert_eq!(std::fs::read(owned).unwrap(), b"abc");
}

#[tokio::test]
async fn empty_replacement_directory_survives_last_stage_removal_race() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("committed");
    let mut process = child(dir.path(), "receipt_committed", &marker);
    wait_marker(&mut process, &marker).await;
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    let hub = app(&dir.path().join("hub")).await;
    recover_transfers(&hub).await.unwrap();
    sqlx::query("DELETE FROM models")
        .execute(&*hub.db)
        .await
        .unwrap();
    db::blob_db::get_and_delete_dead_blobs(&hub.db)
        .await
        .unwrap();
    let marker = dir.path().join("race");
    let parked = dir.path().join("parked-stage");
    let path_file = dir.path().join("replacement-path");
    let mut process = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sync_observation_service::crash_tests::orphan_gc_race_child",
                "--nocapture",
            ])
            .env("MESH_GC_TEST_ROOT", dir.path().join("hub"))
            .env("MESH_GC_SWAP_STAGE", parked)
            .env("MESH_GC_REPLACEMENT_PATH", &path_file)
            .env("MESH_TRANSFER_TEST_POINT", "before_stage_directory_remove")
            .env("MESH_TRANSFER_TEST_MARKER", &marker)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_marker(&mut process, &marker).await;
    assert!(process.0.wait().unwrap().success());
    let replacement = std::fs::read_to_string(path_file).unwrap();
    assert!(
        Path::new(&replacement).is_dir(),
        "an unrelated empty replacement directory must not be removed"
    );
}

#[tokio::test]
async fn cleanup_interrupted_after_contents_clear_remains_recoverable() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("committed");
    let mut process = child(dir.path(), "receipt_committed", &marker);
    wait_marker(&mut process, &marker).await;
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    let hub = app(&dir.path().join("hub")).await;
    recover_transfers(&hub).await.unwrap();
    sqlx::query("DELETE FROM models")
        .execute(&*hub.db)
        .await
        .unwrap();
    db::blob_db::get_and_delete_dead_blobs(&hub.db)
        .await
        .unwrap();
    let cleared = dir.path().join("cleared");
    let mut process = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sync_observation_service::crash_tests::orphan_gc_race_child",
                "--nocapture",
            ])
            .env("MESH_GC_TEST_ROOT", dir.path().join("hub"))
            .env("MESH_TRANSFER_TEST_POINT", "stage_contents_cleared")
            .env("MESH_TRANSFER_TEST_MARKER", &cleared)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_marker(&mut process, &cleared).await;
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transfer_intents")
            .fetch_one(&*hub.db)
            .await
            .unwrap(),
        1
    );
    recover_transfers(&hub).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transfer_intents")
            .fetch_one(&*hub.db)
            .await
            .unwrap(),
        0,
        "recovery must finish cleanup after the process dies"
    );
}

#[tokio::test]
async fn committed_recovery_child() {
    let Ok(root) = std::env::var("MESH_RECOVERY_TEST_ROOT") else {
        return;
    };
    let hub = app(Path::new(&root)).await;
    let result = recover_transfers(&hub).await;
    if std::env::var_os("MESH_RESTORE_FAIL").is_some() {
        assert!(
            result.is_err(),
            "injected restore sync failure must stop recovery"
        );
    } else {
        assert!(
            result.is_ok(),
            "committed recovery must succeed: {result:?}"
        );
    }
    std::fs::write(std::env::var("MESH_RECOVERY_DONE").unwrap(), b"done").unwrap();
}

fn recovery_child(root: &Path, marker: &Path, sync_marker: &Path, fail: bool) -> KillOnDrop {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        "sync_observation_service::crash_tests::committed_recovery_child",
        "--nocapture",
    ])
    .env("MESH_RECOVERY_TEST_ROOT", root)
    .env("MESH_RECOVERY_DONE", marker)
    .env("MESH_RESTORE_SYNC_MARKER", sync_marker)
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    if fail {
        cmd.env("MESH_RESTORE_FAIL", "1");
    }

    KillOnDrop(cmd.spawn().unwrap())
}

#[tokio::test]
async fn committed_quarantine_retains_intent_until_restore_is_durable() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("commit-checkpoint");
    let mut process = child(dir.path(), "receipt_committed", &marker);
    wait_marker(&mut process, &marker).await;
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    let hub_path = dir.path().join("hub");
    let hub = app(&hub_path).await;
    let (stage, manifest): (String, String) =
        sqlx::query_as("SELECT stage_path,manifest FROM sync_transfer_intents")
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    let owned = hub
        .get_model_dir()
        .join(manifest[0]["name"].as_str().unwrap());
    let quarantine = Path::new(&stage).join(".cleanup-0");
    std::fs::rename(&owned, &quarantine).unwrap();
    let failure_done = dir.path().join("failure-done");
    let failure_sync = dir.path().join("failure-sync");
    let mut process = recovery_child(&hub_path, &failure_done, &failure_sync, true);
    wait_marker(&mut process, &failure_done).await;
    assert!(process.0.wait().unwrap().success());
    assert!(
        failure_sync.exists(),
        "recovery must attempt directory sync before unlink"
    );
    assert!(
        owned.exists(),
        "restored root link must survive sync failure"
    );
    assert!(
        quarantine.exists(),
        "sync failure must retain the durable quarantine link"
    );
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_transfer_intents WHERE stage_path=?")
            .bind(&stage)
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    assert_eq!(count, 1, "sync failure must retain recovery intent");
    let success_done = dir.path().join("success-done");
    let success_sync = dir.path().join("success-sync");
    let mut process = recovery_child(&hub_path, &success_done, &success_sync, false);
    wait_marker(&mut process, &success_done).await;
    assert!(process.0.wait().unwrap().success());
    assert!(
        success_sync.exists(),
        "root must be synced before quarantine unlink"
    );
    assert_eq!(std::fs::read(&owned).unwrap(), b"abc");
    assert!(cleaned_stage(Path::new(&stage)));
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_transfer_intents WHERE stage_path=?")
            .bind(&stage)
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn download_capacity_child() {
    let Ok(root) = std::env::var("MESH_DOWNLOAD_TEST_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let state = app(root).await;
    let path = root.join("external.stl");
    std::fs::write(&path, b"abc").unwrap();
    let hash = "ba7816bf8f01cfea414140de5dae2223";
    db::blob_db::add_blob(
        &state.db,
        hash,
        "stl",
        3,
        Some(path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let blob = db::blob_db::get_blob_via_sha256(&state.db, hash)
        .await
        .unwrap()
        .unwrap();
    let content = BlobContent {
        hash: hash.into(),
        filetype: "stl".into(),
        size: 3,
        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
    };
    // More unread downloads than the staging capacity (at most 4): none may pin a slot.
    let mut held = Vec::new();
    for _ in 0..6 {
        held.push(
            prepare_download(&state, &content, &blob)
                .await
                .expect("an unread download must not hold staging capacity"),
        );
    }
    // Starting another staging operation still works while all downloads are unread.
    let _permit = staging_permit(&state).expect("staging capacity must stay free");
    assert!(
        no_payload_stages(&state.get_model_dir()),
        "verified download stages must be removed before streaming"
    );
    for mut download in held {
        let mut bytes = Vec::new();
        AsyncReadExt::read_to_end(&mut download, &mut bytes)
            .await
            .unwrap();
        assert_eq!(
            bytes, b"abc",
            "bytes must stay readable after stage removal"
        );
    }
    std::fs::write(
        std::env::var("MESH_DOWNLOAD_TEST_MARKER").unwrap(),
        b"passed",
    )
    .unwrap();
}

#[tokio::test]
async fn unconsumed_downloads_do_not_hold_staging_capacity() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("download-checked");
    let mut process = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sync_observation_service::crash_tests::download_capacity_child",
                "--nocapture",
            ])
            .env("MESH_DOWNLOAD_TEST_ROOT", dir.path())
            .env("MESH_DOWNLOAD_TEST_MARKER", &marker)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_marker(&mut process, &marker).await;
    assert!(process.0.wait().unwrap().success());
}
