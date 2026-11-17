//! Selected publication records retain their exact proof and content across restarts.
use std::{
    fmt::Write as _,
    sync::{Arc, Mutex},
};

use sha2::Digest;
use tokio::sync::Notify;

use service::sync_pull_service::{
    AuthenticatedHub, HubTrust, PublicationOutcome, discard_queued_publication,
};

use super::{
    pull::{TEST_CA, TlsHub},
    *,
};

fn no_stage_payload(path: &std::path::Path) -> bool {
    path.is_dir()
        && std::fs::read_dir(path).unwrap().all(|entry| {
            let entry = entry.unwrap();
            entry.file_name() == ".lease" && entry.file_type().unwrap().is_file()
        })
}

fn entries_with_payload(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter(|entry| {
            let path = entry.as_ref().unwrap().path();
            !path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".sync-stage-")
                || !no_stage_payload(&path)
        })
        .count()
}

async fn reopen(dir: &std::path::Path) -> service::AppState {
    let data_path = dir.to_string_lossy().into_owned();
    let db = db::db_context::setup_db(&dir.join("db.sqlite"), &dir.join("backups")).await;
    service::AppState {
        db: Arc::new(db),
        configuration: Mutex::new(service::Configuration {
            data_path: data_path.clone(),
            ..Default::default()
        }),
        import_mutex: Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: data_path,
    }
}

async fn legacy_group(state: &service::AppState, user: &db::model::user::User, name: &str) -> i64 {
    sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES(?,'2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(name).bind(user.id).bind(db::random_hex_32()).execute(&*state.db).await.unwrap().last_insert_rowid()
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Exact queued bytes must survive edits, restart, a lost ACK and retry.
async fn queued_observation_survives_source_edits_restart_and_ack_retry() {
    let (_hub_dir, hub, router, _) = app().await;
    let (author_dir, author) = crate::test_support::app_state().await;
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let ai = db::user_db::get_sync_identity(&author.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let ri = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    let author_client = AuthenticatedHub::pair(
        &author,
        &user,
        &ai,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let receiver_client = AuthenticatedHub::pair(
        &receiver,
        &user,
        &ri,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let global = db::random_hex_32();
    let group=sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('original','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(&global).execute(&*author.db).await.unwrap().last_insert_rowid();
    let path = author_dir.path().join("original.stl");
    std::fs::write(&path, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &author.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        Some(path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let model = db::model_db::add_model(&author.db, &user, "part", blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&author.db, &user, Some(group), vec![model], None)
        .await
        .unwrap();
    let queued = author_client
        .queue_observation(&author, &user, &ai, group)
        .await
        .unwrap();
    let staged: String =
        sqlx::query_scalar("SELECT stage_path FROM sync_publication_stages WHERE outbox_id=?")
            .bind(&queued)
            .fetch_one(&*author.db)
            .await
            .unwrap();

    assert!(std::path::Path::new(&staged).exists());
    std::fs::write(&path, b"abd").unwrap();
    db::model_db::edit_model(
        &author.db,
        &user,
        model,
        "part",
        None,
        Some("later description"),
        db::model::ModelFlags::empty(),
        None,
        None,
    )
    .await
    .unwrap();
    drop(author_client);
    author.db.close().await;
    drop(author);
    let author = reopen(author_dir.path()).await;
    let client = AuthenticatedHub::connect(
        &author,
        &user,
        &ai,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert!(
        std::path::Path::new(&staged).exists(),
        "restart must retain queued bytes"
    );
    sqlx::query("CREATE TRIGGER fail_outbox_ack BEFORE UPDATE OF ack_sequence ON sync_publication_outbox BEGIN SELECT RAISE(ABORT,'injected ack failure'); END").execute(&*author.db).await.unwrap();
    assert!(
        client
            .publish_queued(&author, &user, &ai, &queued)
            .await
            .is_err()
    );
    let pending: Option<String> =
        sqlx::query_scalar("SELECT ack_sequence FROM sync_publication_outbox WHERE outbox_id=?")
            .bind(&queued)
            .fetch_one(&*author.db)
            .await
            .unwrap();
    assert_eq!(pending, None);
    assert!(
        std::path::Path::new(&staged).exists(),
        "failed acknowledgement must retain the staged bytes"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap(),
        1,
        "hub may have committed before local ACK failure"
    );
    sqlx::query("DROP TRIGGER fail_outbox_ack")
        .execute(&*author.db)
        .await
        .unwrap();
    assert_eq!(
        client
            .publish_queued(&author, &user, &ai, &queued)
            .await
            .unwrap(),
        PublicationOutcome::AlreadyPresent {
            sequence: "1".into()
        }
    );
    let acknowledged: Option<String> =
        sqlx::query_scalar("SELECT ack_sequence FROM sync_publication_outbox WHERE outbox_id=?")
            .bind(&queued)
            .fetch_one(&*author.db)
            .await
            .unwrap();
    assert_eq!(acknowledged.as_deref(), Some("1"));
    let item: Option<String> =
        sqlx::query_scalar("SELECT item FROM sync_publication_outbox WHERE outbox_id=?")
            .bind(&queued)
            .fetch_one(&*author.db)
            .await
            .unwrap();
    assert!(
        item.is_none(),
        "an acknowledged item must release its bulky payload while retaining its receipt"
    );
    assert_eq!(
        client
            .publish_queued(&author, &user, &ai, &queued)
            .await
            .unwrap(),
        PublicationOutcome::AlreadyPresent {
            sequence: "1".into()
        },
        "a compacted receipt must remain idempotent"
    );
    assert_eq!(
        receiver_client
            .pull_next(&receiver, &user, &ri)
            .await
            .unwrap(),
        service::sync_pull_service::PullOutcome::Applied {
            sequence: "1".into()
        }
    );
    let model_row:(String,Option<String>,i64)=sqlx::query_as("SELECT m.model_name,m.model_desc,m.model_blob_id FROM models m JOIN models_group g ON g.group_id=m.model_group_id WHERE g.group_user_id=? AND g.group_unique_global_id=?").bind(user.id).bind(global).fetch_one(&*receiver.db).await.unwrap();
    assert_eq!(model_row.0, "part");
    assert_eq!(model_row.1, None);
    let blob = db::blob_db::get_blobs_via_ids(&receiver.db, vec![model_row.2])
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(
        std::fs::read(service::export_service::get_model_path_for_blob(
            &blob, &receiver
        ))
        .unwrap(),
        b"abc"
    );
}

#[tokio::test]
async fn queued_operation_with_missing_parent_remains_pending_until_parent_is_published() {
    let (_hub_dir, hub, router, _) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    let client = AuthenticatedHub::pair(
        &source,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let group = db::group_db::add_empty_group(&source.db, &user, "original", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    let genesis = client
        .queue_genesis(&source, &user, &identity, &root)
        .await
        .unwrap();
    db::group_db::edit_group(&source.db, &user, group.id, "renamed", None, None)
        .await
        .unwrap();
    let NameState::Ready { token, .. } =
        db::causal_group_lifecycle::get_state(&source.db, &user, &identity, group.id)
            .await
            .unwrap()
            .name
    else {
        panic!("expected rename")
    };
    let operation = client
        .queue_operation(&source, &user, &identity, &token.frontier[0])
        .await
        .unwrap();
    assert!(
        matches!(client.publish_queued(&source,&user,&identity,&operation).await.unwrap(),PublicationOutcome::NeedsDependencies {ref missing} if !missing.is_empty())
    );
    let pending: Option<String> =
        sqlx::query_scalar("SELECT ack_sequence FROM sync_publication_outbox WHERE outbox_id=?")
            .bind(&operation)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    assert_eq!(pending, None);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        client
            .publish_queued(&source, &user, &identity, &genesis)
            .await
            .unwrap(),
        PublicationOutcome::Published {
            sequence: "1".into()
        }
    );
    assert_eq!(
        client
            .publish_queued(&source, &user, &identity, &operation)
            .await
            .unwrap(),
        PublicationOutcome::Published {
            sequence: "2".into()
        }
    );
}

#[tokio::test]
async fn queued_item_cannot_publish_under_a_replaced_local_link_or_account() {
    let (_hub_dir, hub, router, _) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let (_other_dir, other) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = db::user_db::get_sync_identity(&other.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    let client = AuthenticatedHub::pair(
        &source,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let group = db::group_db::add_empty_group(&source.db, &user, "folder", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    let queued = client
        .queue_genesis(&source, &user, &identity, &root)
        .await
        .unwrap();
    assert!(
        client
            .publish_queued(&other, &user, &other_identity, &queued)
            .await
            .is_err(),
        "other local account must not publish a queued record"
    );
    db::user_db::set_user_sync_token(
        &source.db,
        user.id,
        &db::random_hex_32(),
        &server.origin(),
        false,
    )
    .await
    .unwrap();
    db::user_db::set_user_sync_token(&source.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    assert!(
        client
            .publish_queued(&source, &user, &identity, &queued)
            .await
            .is_err(),
        "link ABA must invalidate the client"
    );
    let pending: Option<String> =
        sqlx::query_scalar("SELECT ack_sequence FROM sync_publication_outbox WHERE outbox_id=?")
            .bind(&queued)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    assert_eq!(pending, None);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Source enrollment and durable queue insertion must fail as one transaction.
async fn failed_queue_insert_leaves_no_enrollment_or_staged_files() {
    let (_hub_dir, hub, router, _) = app().await;
    let (source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    let client = AuthenticatedHub::pair(
        &source,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let group = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('original','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(db::random_hex_32()).execute(&*source.db).await.unwrap().last_insert_rowid();
    let path = source_dir.path().join("original.stl");
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
    let before:(i64,i64,i64,Option<String>,Option<String>)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM causal_observation_batches),(SELECT counter FROM causal_account_writers WHERE user_id=?),(SELECT group_birth_id FROM models_group WHERE group_id=?),(SELECT model_birth_id FROM models WHERE model_id=?)").bind(user.id).bind(group).bind(model).fetch_one(&*source.db).await.unwrap();
    let dir = source.get_model_dir();
    let files = entries_with_payload(&dir);
    sqlx::query("CREATE TRIGGER fail_outbox_insert BEFORE INSERT ON sync_publication_outbox BEGIN SELECT RAISE(ABORT,'injected queue failure'); END").execute(&*source.db).await.unwrap();
    assert!(
        client
            .queue_observation(&source, &user, &identity, group)
            .await
            .is_err()
    );
    let after:(i64,i64,i64,Option<String>,Option<String>)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM causal_observation_batches),(SELECT counter FROM causal_account_writers WHERE user_id=?),(SELECT group_birth_id FROM models_group WHERE group_id=?),(SELECT model_birth_id FROM models WHERE model_id=?)").bind(user.id).bind(group).bind(model).fetch_one(&*source.db).await.unwrap();
    assert_eq!(
        before, after,
        "failed queue insertion must roll back enrollment and the writer clock"
    );
    assert_eq!(
        entries_with_payload(&dir),
        files,
        "failed queue insertion must clean its staging"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_publication_outbox")
            .fetch_one(&*source.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Re-selecting a rekeyed group must not silently return its historical publication.
async fn rekeyed_group_cannot_select_its_old_observation_as_current() {
    let (_hub_dir, hub, router, _) = app().await;
    let (source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    let client = AuthenticatedHub::pair(
        &source,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let original = db::random_hex_32();
    let group=sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('original','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(&original).execute(&*source.db).await.unwrap().last_insert_rowid();
    let file = source_dir.path().join("original.stl");
    std::fs::write(&file, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &source.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        Some(file.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let model = db::model_db::add_model(&source.db, &user, "part", blob, None, None)
        .await
        .unwrap();
    // Reproduce an imported legacy membership with no user-move history.
    sqlx::query("UPDATE models SET model_group_id=? WHERE model_id=? AND model_user_id=?")
        .bind(group)
        .bind(model)
        .bind(user.id)
        .execute(&*source.db)
        .await
        .unwrap();
    let old = client
        .queue_observation(&source, &user, &identity, group)
        .await
        .unwrap();
    let replacement = db::random_hex_32();
    db::group_db::edit_group(
        &source.db,
        &user,
        group,
        "original",
        None,
        Some(&replacement),
    )
    .await
    .unwrap();
    let actual: String =
        sqlx::query_scalar("SELECT group_unique_global_id FROM models_group WHERE group_id=?")
            .bind(group)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    assert_eq!(actual, replacement);
    let current = client
        .queue_observation(&source, &user, &identity, group)
        .await;
    assert!(
        !matches!(current,Ok(ref id) if id==&old),
        "rekeying must not reselect an old global identity"
    );
    assert_eq!(
        client
            .publish_queued(&source, &user, &identity, &old)
            .await
            .unwrap(),
        PublicationOutcome::Published {
            sequence: "1".into()
        },
        "explicit historical outbox item remains publishable"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Explicit discard is owner-bound and releases only certified queued storage, even after unlink.
async fn explicit_discard_after_unlink_preserves_original_data_and_history() {
    let (_hub_dir, hub, router, _) = app().await;
    let (source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    let client = AuthenticatedHub::pair(
        &source,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let group=sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('original','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(db::random_hex_32()).execute(&*source.db).await.unwrap().last_insert_rowid();
    let file = source_dir.path().join("original.stl");
    std::fs::write(&file, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &source.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        Some(file.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let model = db::model_db::add_model(&source.db, &user, "part", blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&source.db, &user, Some(group), vec![model], None)
        .await
        .unwrap();
    let queued = client
        .queue_observation(&source, &user, &identity, group)
        .await
        .unwrap();
    let staged: String =
        sqlx::query_scalar("SELECT stage_path FROM sync_publication_stages WHERE outbox_id=?")
            .bind(&queued)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    let clone_dir = tempfile::tempdir().unwrap();
    let clone_path = clone_dir.path().join("db.sqlite");
    sqlx::query("VACUUM INTO ?")
        .bind(clone_path.to_string_lossy().into_owned())
        .execute(&*source.db)
        .await
        .unwrap();
    let backups = clone_dir.path().join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    let clone_db = db::db_context::setup_db(&clone_path, &backups).await;
    let cloned = service::AppState {
        db: Arc::new(clone_db),
        configuration: Mutex::new(source.get_configuration()),
        import_mutex: Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: source.app_data_path.clone(),
    };
    let original_contents: Vec<_> = std::fs::read_dir(&staged)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (path.clone(), std::fs::read(path).unwrap())
        })
        .collect();
    let slots_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_stage_slots")
        .fetch_one(&*cloned.db)
        .await
        .unwrap();
    assert!(
        discard_queued_publication(&cloned, &user, &identity, &queued)
            .await
            .is_err(),
        "a copied database cannot discard a foreign stage without proof of ownership"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE ack_sequence IS NULL"
        )
        .fetch_one(&*cloned.db)
        .await
        .unwrap(),
        1,
        "a blocked discard retains the cloned pending record"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_stage_slots")
            .fetch_one(&*cloned.db)
            .await
            .unwrap(),
        slots_before,
        "foreign slot remains counted"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE outbox_id=?"
        )
        .bind(&queued)
        .fetch_one(&*source.db)
        .await
        .unwrap(),
        1,
        "original queue remains intact"
    );
    for (path, bytes) in original_contents {
        assert_eq!(
            std::fs::read(path).unwrap(),
            bytes,
            "discard cannot touch original stage files"
        );
    }
    let original_db = db::sync_transport::stage_slots::database_path(&source.db)
        .await
        .unwrap();
    let parked_db = source_dir.path().join("parked-db.sqlite");
    std::fs::rename(&original_db, &parked_db).unwrap();
    std::fs::copy(&parked_db, &original_db).unwrap();
    let replacement_result = discard_queued_publication(&source, &user, &identity, &queued).await;
    std::fs::remove_file(&original_db).unwrap();
    std::fs::rename(&parked_db, &original_db).unwrap();
    assert!(
        replacement_result.is_err(),
        "a replaced DB pathname must not certify a live connection as foreign"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE outbox_id=?"
        )
        .bind(&queued)
        .fetch_one(&*source.db)
        .await
        .unwrap(),
        1
    );
    assert!(
        !no_stage_payload(std::path::Path::new(&staged)),
        "the original pending bytes must remain"
    );
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events WHERE user_id=?")
        .bind(user.id)
        .fetch_one(&*source.db)
        .await
        .unwrap();
    let other_id = db::user_db::add_user(
        &source.db,
        "other",
        "other@example.com",
        &db::random_hex_32(),
    )
    .await
    .unwrap();
    let other = db::user_db::get_user_by_id(&source.db, other_id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = db::user_db::get_sync_identity(&source.db, other_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        discard_queued_publication(&source, &other, &other_identity, &queued)
            .await
            .is_err(),
        "another owner cannot discard the queue"
    );
    assert!(std::path::Path::new(&staged).exists());
    db::user_db::clear_user_sync(&source.db, user.id)
        .await
        .unwrap();
    discard_queued_publication(&source, &user, &identity, &queued)
        .await
        .unwrap();
    assert!(
        no_stage_payload(std::path::Path::new(&staged)),
        "discard must clear all staged payload and lease files"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE outbox_id=?"
        )
        .bind(&queued)
        .fetch_one(&*source.db)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM causal_events WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*source.db)
            .await
            .unwrap(),
        events
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"abc");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM models WHERE model_id=? AND model_user_id=?"
        )
        .bind(model)
        .bind(user.id)
        .fetch_one(&*source.db)
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Two full-size logical blobs exercise the real aggregate capacity boundary.
async fn aggregate_pending_quota_is_atomic_and_discard_releases_capacity() {
    let (_hub_dir, hub, router, _) = app().await;
    let (source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    let client = AuthenticatedHub::pair(
        &source,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let big_group = legacy_group(&source, &user, "large").await;
    let mut bytes = vec![0_u8; 32 * 1024 * 1024];
    for index in 0..2 {
        bytes.fill(index);
        let digest = sha2::Sha256::digest(&bytes)
            .iter()
            .fold(String::new(), |mut out, byte| {
                write!(out, "{byte:02x}").unwrap();
                out
            });
        let path = source_dir.path().join(format!("large-{index}.stl"));
        std::fs::write(&path, &bytes).unwrap();
        let blob = db::blob_db::add_blob(
            &source.db,
            &digest[..32],
            "stl",
            i64::try_from(bytes.len()).unwrap(),
            Some(path.to_string_lossy().into_owned()),
        )
        .await
        .unwrap();
        let model = db::model_db::add_model(
            &source.db,
            &user,
            &format!("part-{index}"),
            blob,
            None,
            None,
        )
        .await
        .unwrap();
        db::group_db::set_group_id_on_models(&source.db, &user, Some(big_group), vec![model], None)
            .await
            .unwrap();
    }
    drop(bytes);
    let full = client
        .queue_observation(&source, &user, &identity, big_group)
        .await
        .unwrap();
    let size: i64 =
        sqlx::query_scalar("SELECT logical_bytes FROM sync_publication_stages WHERE outbox_id=?")
            .bind(&full)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    assert_eq!(size, 64 * 1024 * 1024);
    let small_group = legacy_group(&source, &user, "small").await;
    let small_path = source_dir.path().join("small.stl");
    std::fs::write(&small_path, b"abc").unwrap();
    let small_blob = db::blob_db::add_blob(
        &source.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        Some(small_path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let small_model = db::model_db::add_model(&source.db, &user, "small", small_blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(
        &source.db,
        &user,
        Some(small_group),
        vec![small_model],
        None,
    )
    .await
    .unwrap();
    let before:(i64,i64,Option<String>,Option<String>)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM sync_publication_outbox),(SELECT COUNT(*) FROM causal_events),(SELECT group_birth_id FROM models_group WHERE group_id=?),(SELECT model_birth_id FROM models WHERE model_id=?)").bind(small_group).bind(small_model).fetch_one(&*source.db).await.unwrap();
    let stages_before = entries_with_payload(&source.get_model_dir());
    assert!(
        client
            .queue_observation(&source, &user, &identity, small_group)
            .await
            .is_err(),
        "pending logical-byte cap must reject the next b"
    );
    let after:(i64,i64,Option<String>,Option<String>)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM sync_publication_outbox),(SELECT COUNT(*) FROM causal_events),(SELECT group_birth_id FROM models_group WHERE group_id=?),(SELECT model_birth_id FROM models WHERE model_id=?)").bind(small_group).bind(small_model).fetch_one(&*source.db).await.unwrap();
    assert_eq!(
        before, after,
        "quota rejection cannot partially enroll the second b"
    );
    assert_eq!(entries_with_payload(&source.get_model_dir()), stages_before);
    db::user_db::clear_user_sync(&source.db, user.id)
        .await
        .unwrap();
    discard_queued_publication(&source, &user, &identity, &full)
        .await
        .unwrap();
    let client = AuthenticatedHub::pair(
        &source,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let small = client
        .queue_observation(&source, &user, &identity, small_group)
        .await
        .unwrap();
    let size: i64 =
        sqlx::query_scalar("SELECT logical_bytes FROM sync_publication_stages WHERE outbox_id=?")
            .bind(&small)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    assert_eq!(size, 3);
    assert_eq!(std::fs::read(small_path).unwrap(), b"abc");
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // A replaced stage must survive cleanup after the hub commits the queued item.
async fn acknowledged_outbox_never_deletes_a_replacement_stage_directory() {
    let (_hub_dir, hub, router, _) = app().await;
    let (source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let reached = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let (arrived, proceed) = (reached.clone(), release.clone());
    let router = router.layer(axum::middleware::from_fn(
        move |request: Request<Body>, next: axum::middleware::Next| {
            let (arrived, proceed) = (arrived.clone(), proceed.clone());
            async move {
                let upload = request.uri().path() == "/api/v2/sync/observations"
                    && request.method() == axum::http::Method::POST;
                let response = next.run(request).await;
                if upload {
                    arrived.notify_one();
                    proceed.notified().await;
                }
                response
            }
        },
    ));
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    let client = AuthenticatedHub::pair(
        &source,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let group = legacy_group(&source, &user, "folder").await;
    let file = source_dir.path().join("original.stl");
    std::fs::write(&file, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &source.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        Some(file.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let model = db::model_db::add_model(&source.db, &user, "part", blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&source.db, &user, Some(group), vec![model], None)
        .await
        .unwrap();
    let queued = client
        .queue_observation(&source, &user, &identity, group)
        .await
        .unwrap();
    let staged: String =
        sqlx::query_scalar("SELECT stage_path FROM sync_publication_stages WHERE outbox_id=?")
            .bind(&queued)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    let parked = source_dir.path().join("parked-stage");
    let publish = client.publish_queued(&source, &user, &identity, &queued);
    let replace = async {
        reached.notified().await;
        std::fs::rename(&staged, &parked).unwrap();
        std::fs::create_dir(&staged).unwrap();
        std::fs::write(std::path::Path::new(&staged).join("unrelated.stl"), b"keep").unwrap();
        release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        Box::pin(async { tokio::join!(publish, replace) }),
    )
    .await
    .expect("upload must reach committed-response barrier");
    assert_eq!(
        std::fs::read(std::path::Path::new(&staged).join("unrelated.stl")).unwrap(),
        b"keep"
    );
    assert!(
        result.is_err(),
        "replaced stage cannot be treated as safely cleaned"
    );
    let intent: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_transfer_intents WHERE stage_path=?")
            .bind(&staged)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    assert_eq!(intent, 1, "identity mismatch must retain cleanup evidence");
    assert_eq!(std::fs::read(file).unwrap(), b"abc");
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // A copied lease token and valid bytes cannot certify a replaced directory.
async fn queued_outbox_rejects_a_replaced_stage_even_with_copied_lease() {
    let (_hub_dir, hub, router, _) = app().await;
    let (source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    let client = AuthenticatedHub::pair(
        &source,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let group = legacy_group(&source, &user, "folder").await;
    let file = source_dir.path().join("original.stl");
    std::fs::write(&file, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &source.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        Some(file.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let model = db::model_db::add_model(&source.db, &user, "part", blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&source.db, &user, Some(group), vec![model], None)
        .await
        .unwrap();
    let queued = client
        .queue_observation(&source, &user, &identity, group)
        .await
        .unwrap();
    let staged: String =
        sqlx::query_scalar("SELECT stage_path FROM sync_publication_stages WHERE outbox_id=?")
            .bind(&queued)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    let parked = source_dir.path().join("parked-stage");
    std::fs::rename(&staged, &parked).unwrap();
    std::fs::create_dir(&staged).unwrap();
    for entry in std::fs::read_dir(&parked).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(
            entry.path(),
            std::path::Path::new(&staged).join(entry.file_name()),
        )
        .unwrap();
    }
    let unrelated = std::path::Path::new(&staged).join("unrelated.stl");
    std::fs::write(&unrelated, b"keep").unwrap();
    assert!(
        client
            .publish_queued(&source, &user, &identity, &queued)
            .await
            .is_err(),
        "copied lease cannot authorize a different directory"
    );
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep");
    let ack: Option<String> =
        sqlx::query_scalar("SELECT ack_sequence FROM sync_publication_outbox WHERE outbox_id=?")
            .bind(&queued)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    assert_eq!(ack, None);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap(),
        0
    );
}
