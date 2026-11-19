//! User-confirmed identity matching must preserve independent legacy histories.
use std::{
    fmt::Write,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::response::IntoResponse;
use sha2::Digest;

use db::{
    causal_envelope::{Action, canonical_causal_envelope, parse_causal_envelope},
    causal_group_lifecycle::GroupLifecycle,
    causal_group_rename::{NameState, ResolveResult},
    causal_model_move::MoveState,
    causal_store::IdentityStatus,
};
use service::sync_pull_service::{
    AdvanceOutcome, AuthenticatedHub, GroupIdentityDecision, HubTrust, ModelIdentityDecision,
    NamePublication, OperationSelection, discard_queued_publication,
};

use super::{
    pull::{TEST_CA, TlsHub},
    *,
};

struct Fixture {
    dirs: Vec<tempfile::TempDir>,
    server: TlsHub,
    receiver: service::AppState,
    author: service::AppState,
    hub: service::AppState,
    user: db::model::user::User,
    receiver_identity: db::user_db::SyncIdentity,
    author_identity: db::user_db::SyncIdentity,
    client: AuthenticatedHub,
    sender: AuthenticatedHub,
    local: i64,
    remote: i64,
    global: String,
    token: String,
    child: Option<(String, String)>,
}

async fn fixture() -> Fixture {
    fixture_with_remote_model(false).await
}

async fn add_imported_local_model(fixture: &Fixture) -> (i64, std::path::PathBuf) {
    let path = fixture.dirs[2].path().join("original-local.stl");
    std::fs::write(&path, b"xyz").unwrap();
    let blob = db::blob_db::add_blob(
        &fixture.receiver.db,
        "3608bca1e44ea6c4d268eb6db0226026",
        "stl",
        3,
        Some(path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let model = db::model_db::add_model(
        &fixture.receiver.db,
        &fixture.user,
        "local model",
        blob,
        None,
        None,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE models SET model_group_id=? WHERE model_id=?")
        .bind(fixture.local)
        .bind(model)
        .execute(&*fixture.receiver.db)
        .await
        .unwrap();
    (model, path)
}

async fn fixture_with_remote_model(populated: bool) -> Fixture {
    fixture_with_download_barrier(populated, None, false).await
}

async fn received_name_fixture(populated: bool) -> (Fixture, String) {
    let fixture = fixture_with_remote_model(populated).await;
    if populated {
        add_imported_local_model(&fixture).await;
    }
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    let linked = fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap();
    fixture
        .client
        .publish_queued(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &linked.outbox_id.unwrap(),
        )
        .await
        .unwrap();
    fixture
        .sender
        .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap();
    fixture
        .sender
        .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap();
    (fixture, choice.decision_id)
}

#[allow(clippy::too_many_lines)] // The fixture builds authenticated peers and optionally controls the blob response.
async fn fixture_with_download_barrier(
    populated: bool,
    barrier: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    corrupt: bool,
) -> Fixture {
    fixture_with_counter(FixtureOptions {
        populated,
        barrier,
        corrupt,
        ..FixtureOptions::default()
    })
    .await
}

type FeedGate = (
    Arc<std::sync::atomic::AtomicBool>,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
);

// Named knobs so adjacent `bool`/`Option` arguments cannot be swapped silently.
#[derive(Default)]
struct FixtureOptions {
    populated: bool,
    barrier: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    corrupt: bool,
    counter: Option<Arc<AtomicUsize>>,
    feed_gate: Option<FeedGate>,
    attack: Option<Arc<AtomicUsize>>,
}

#[allow(clippy::too_many_lines)] // The fixture builds authenticated peers and optionally controls and counts responses.
async fn fixture_with_counter(options: FixtureOptions) -> Fixture {
    let FixtureOptions {
        populated,
        barrier,
        corrupt,
        counter,
        feed_gate,
        attack,
    } = options;
    let (hub_dir, hub, router, _) = app().await;
    let (author_dir, author) = crate::test_support::app_state().await;
    let (receiver_dir, receiver) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let author_identity = db::user_db::get_sync_identity(&author.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let receiver_identity = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let router = if let Some((reached, release)) = barrier {
        router.layer(axum::middleware::from_fn(
            move |request: Request<Body>, next: axum::middleware::Next| {
                let (reached, release) = (reached.clone(), release.clone());
                async move {
                    if request.uri().path().contains("/blobs/") {
                        reached.notify_one();
                        release.notified().await;
                    }
                    next.run(request).await
                }
            },
        ))
    } else {
        router
    };
    let router = if corrupt {
        router.layer(axum::middleware::from_fn(
            |request: Request<Body>, next: axum::middleware::Next| async move {
                if request.uri().path().contains("/blobs/") {
                    return (StatusCode::OK, "abd").into_response();
                }
                next.run(request).await
            },
        ))
    } else {
        router
    };
    let router = if let Some(counter) = counter {
        router.layer(axum::middleware::from_fn(
            move |request: Request<Body>, next: axum::middleware::Next| {
                let counter = counter.clone();
                async move {
                    if request.method() == axum::http::Method::GET
                        && request.uri().path() == "/api/v2/sync/items"
                    {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                    next.run(request).await
                }
            },
        ))
    } else {
        router
    };
    let router = if let Some((armed, reached, release)) = feed_gate {
        router.layer(axum::middleware::from_fn(
            move |request: Request<Body>, next: axum::middleware::Next| {
                let (armed, reached, release) = (armed.clone(), reached.clone(), release.clone());
                async move {
                    let is_feed = request.method() == axum::http::Method::GET
                        && request.uri().path() == "/api/v2/sync/items";
                    let response = next.run(request).await;
                    if is_feed && armed.load(Ordering::SeqCst) {
                        reached.notify_one();
                        release.notified().await;
                    }
                    response
                }
            },
        ))
    } else {
        router
    };
    let router = if let Some(attack) = attack {
        router.layer(axum::middleware::from_fn(
            move |request: Request<Body>, next: axum::middleware::Next| {
                let attack = attack.clone();
                async move {
                    let is_feed = request.method() == axum::http::Method::GET
                        && request.uri().path() == "/api/v2/sync/items";
                    let response = next.run(request).await;
                    let mode = attack.load(Ordering::SeqCst);
                    if !is_feed || mode == 0 {
                        return response;
                    }
                    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
                        .await
                        .unwrap();
                    let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    match mode {
                        1 => json["items"][0]["digest"] = "0".repeat(64).into(),
                        2 => json["items"][0]["sequence"] = "2".into(),
                        3 => json["unexpected"] = true.into(),
                        _ => unreachable!(),
                    }
                    axum::Json(json).into_response()
                }
            },
        ))
    } else {
        router
    };
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    let sender = AuthenticatedHub::pair(
        &author,
        &user,
        &author_identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let client = AuthenticatedHub::pair(
        &receiver,
        &user,
        &receiver_identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let global = db::random_hex_32();
    let insert = "INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES(?,'2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)";
    let remote = sqlx::query(insert)
        .bind("remote name")
        .bind(user.id)
        .bind(&global)
        .execute(&*author.db)
        .await
        .unwrap()
        .last_insert_rowid();
    let local = sqlx::query(insert)
        .bind("local name")
        .bind(user.id)
        .bind(&global)
        .execute(&*receiver.db)
        .await
        .unwrap()
        .last_insert_rowid();
    let child_id = if populated {
        let path = author_dir.path().join("identity-child.stl");
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
        let model = db::model_db::add_model(&author.db, &user, "remote child", blob, None, None)
            .await
            .unwrap();
        sqlx::query("UPDATE models SET model_group_id=? WHERE model_id=?")
            .bind(remote)
            .bind(model)
            .execute(&*author.db)
            .await
            .unwrap();
        Some(model)
    } else {
        None
    };
    let queued = sender
        .queue_observation(&author, &user, &author_identity, remote)
        .await
        .unwrap();
    sender
        .publish_queued(&author, &user, &author_identity, &queued)
        .await
        .unwrap();
    let child = if let Some(model) = child_id {
        Some(
            sqlx::query_as::<_, (String, String)>(
                "SELECT model_unique_global_id,model_birth_id FROM models WHERE model_id=?",
            )
            .bind(model)
            .fetch_one(&*author.db)
            .await
            .unwrap(),
        )
    } else {
        None
    };
    Fixture {
        dirs: vec![hub_dir, author_dir, receiver_dir],
        server,
        receiver,
        author,
        hub,
        user,
        receiver_identity,
        author_identity,
        client,
        sender,
        local,
        remote,
        global,
        token,
        child,
    }
}

#[tokio::test]
async fn confirmed_empty_legacy_identity_preserves_both_names_and_unknown_roots() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture().await;

    let choice = client
        .prepare_group_identity(&receiver, &user, &receiver_identity, local)
        .await
        .unwrap();
    assert_eq!(
        (&*choice.local_name, &*choice.remote_name),
        ("local name", "remote name")
    );
    let before: (String, Option<String>) =
        sqlx::query_as("SELECT group_name,group_birth_id FROM models_group WHERE group_id=?")
            .bind(local)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    assert_eq!(
        before,
        ("local name".into(), None),
        "preparing must not merge groups"
    );
    let result = client
        .confirm_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
        .await
        .unwrap();
    assert_eq!(result.group_id, local);
    assert!(!result.already_present);
    let retry = client
        .confirm_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
        .await
        .unwrap();
    assert!(retry.already_present);
    assert_eq!(retry.event, result.event);
    let NameState::Conflict { choices, .. } =
        db::causal_group_lifecycle::get_state(&receiver.db, &user, &receiver_identity, local)
            .await
            .unwrap()
            .name
    else {
        panic!("both independently observed names must be offered")
    };
    let mut names: Vec<_> = choices.iter().map(|choice| choice.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["local name", "remote name"]);
    let roots: Vec<String> = sqlx::query_scalar("SELECT envelope FROM causal_events WHERE user_id=? AND json_extract(envelope,'$.action.type')='legacy_observed'").bind(user.id).fetch_all(&*receiver.db).await.unwrap();
    assert_eq!(
        roots.len(),
        2,
        "both original observations must be retained"
    );
    for root in roots {
        let event = parse_causal_envelope(&root).unwrap();
        assert!(
            event.parents.is_empty(),
            "neither observation invents earlier ancestry"
        );
    }
}

#[tokio::test]
async fn stale_or_foreign_confirmation_never_adopts_or_advances_the_feed() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture().await;
    let choice = client
        .prepare_group_identity(&receiver, &user, &receiver_identity, local)
        .await
        .unwrap();
    let cursor_before: Vec<(String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT stream_id,last_applied,through FROM sync_pull_streams ORDER BY stream_id",
    )
    .fetch_all(&*receiver.db)
    .await
    .unwrap();
    let other_id = db::user_db::add_user(
        &receiver.db,
        "other",
        "other-identity@example.com",
        &db::random_hex_32(),
    )
    .await
    .unwrap();
    let other = db::user_db::get_user_by_id(&receiver.db, other_id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = db::user_db::get_sync_identity(&receiver.db, other_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        client
            .confirm_group_identity(&receiver, &other, &other_identity, &choice.decision_id)
            .await
            .is_err()
    );
    db::group_db::edit_group(&receiver.db, &user, local, "changed", None, None)
        .await
        .unwrap();
    assert!(
        client
            .confirm_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
            .await
            .is_err(),
        "an older decision cannot overwrite a local edit"
    );
    let row: (String, Option<String>) =
        sqlx::query_as("SELECT group_name,group_birth_id FROM models_group WHERE group_id=?")
            .bind(local)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    assert_eq!(row, ("changed".into(), None));
    db::group_db::edit_group(&receiver.db, &user, local, "local name", None, None)
        .await
        .unwrap();
    assert!(
        client
            .confirm_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
            .await
            .is_err(),
        "returning to the original name must not revive a stale decision"
    );
    let cursor_after: Vec<(String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT stream_id,last_applied,through FROM sync_pull_streams ORDER BY stream_id",
    )
    .fetch_all(&*receiver.db)
    .await
    .unwrap();
    assert_eq!(cursor_before, cursor_after);
}

#[tokio::test]
async fn different_global_identity_is_not_offered_as_a_match() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        global,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture().await;
    sqlx::query("UPDATE models_group SET group_unique_global_id=? WHERE group_id=?")
        .bind(db::random_hex_32())
        .bind(local)
        .execute(&*receiver.db)
        .await
        .unwrap();
    assert!(
        client
            .prepare_group_identity(&receiver, &user, &receiver_identity, local)
            .await
            .is_err()
    );
    let birth: Option<String> =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id=?")
            .bind(local)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    assert_eq!(birth, None);
    assert_ne!(
        sqlx::query_scalar::<_, String>(
            "SELECT group_unique_global_id FROM models_group WHERE group_id=?"
        )
        .bind(local)
        .fetch_one(&*receiver.db)
        .await
        .unwrap(),
        global
    );
}

#[tokio::test]
async fn ambiguous_duplicate_or_known_birth_cannot_be_adopted() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        global,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture().await;
    let duplicate = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('duplicate','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)")
        .bind(user.id).bind(&global).execute(&*receiver.db).await.unwrap().last_insert_rowid();
    assert!(
        client
            .prepare_group_identity(&receiver, &user, &receiver_identity, local)
            .await
            .is_err(),
        "duplicate local candidates require a separate resolution"
    );
    sqlx::query("DELETE FROM models_group WHERE group_id=?")
        .bind(duplicate)
        .execute(&*receiver.db)
        .await
        .unwrap();
    sqlx::query("UPDATE models_group SET group_unique_global_id=? WHERE group_id=?")
        .bind(db::random_hex_32())
        .bind(local)
        .execute(&*receiver.db)
        .await
        .unwrap();
    let known = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id,group_birth_id) VALUES('known','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?,?)")
        .bind(user.id).bind(&global).bind(db::random_hex_32()).execute(&*receiver.db).await.unwrap().last_insert_rowid();
    assert!(
        client
            .prepare_group_identity(&receiver, &user, &receiver_identity, known)
            .await
            .is_err(),
        "known births cannot be reassigned through legacy adoption"
    );
}

#[tokio::test]
async fn deletion_record_prevents_identity_adoption() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        global,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture().await;
    db::group_db::delete_group(&receiver.db, &user, local)
        .await
        .unwrap();
    let replacement = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('replacement','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)")
        .bind(user.id).bind(&global).execute(&*receiver.db).await.unwrap().last_insert_rowid();
    assert!(
        client
            .prepare_group_identity(&receiver, &user, &receiver_identity, replacement)
            .await
            .is_err(),
        "identity with a deletion record must not be adopted"
    );
    let birth: Option<String> =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id=?")
            .bind(replacement)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    assert_eq!(birth, None);
}

#[tokio::test]
async fn populated_or_project_associated_targets_are_not_silently_linked() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture().await;
    let project = db::resource_db::add_resource(&receiver.db, &user, "project", None)
        .await
        .unwrap();
    sqlx::query("UPDATE models_group SET group_resource_id=? WHERE group_id=?")
        .bind(project.id)
        .bind(local)
        .execute(&*receiver.db)
        .await
        .unwrap();
    assert!(
        client
            .prepare_group_identity(&receiver, &user, &receiver_identity, local)
            .await
            .is_err()
    );
    sqlx::query("UPDATE models_group SET group_resource_id=NULL WHERE group_id=?")
        .bind(local)
        .execute(&*receiver.db)
        .await
        .unwrap();
    let blob = db::blob_db::add_blob(&receiver.db, &db::random_hex_32(), "stl", 3, None)
        .await
        .unwrap();
    let model = db::model_db::add_model(&receiver.db, &user, "part", blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&receiver.db, &user, Some(local), vec![model], None)
        .await
        .unwrap();
    assert!(
        client
            .prepare_group_identity(&receiver, &user, &receiver_identity, local)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(model)
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        local
    );
}

#[tokio::test]
async fn failed_receipt_commit_rolls_back_identity_and_allows_retry() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture().await;
    let choice = client
        .prepare_group_identity(&receiver, &user, &receiver_identity, local)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_identity_receipt BEFORE INSERT ON sync_pull_receipts BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END").execute(&*receiver.db).await.unwrap();
    assert!(
        client
            .confirm_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
            .await
            .is_err()
    );
    let row: (String, Option<String>) =
        sqlx::query_as("SELECT group_name,group_birth_id FROM models_group WHERE group_id=?")
            .bind(local)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    assert_eq!(row, ("local name".into(), None));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM causal_events")
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER fail_identity_receipt")
        .execute(&*receiver.db)
        .await
        .unwrap();
    assert!(
        !client
            .confirm_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
            .await
            .unwrap()
            .already_present
    );
}

#[tokio::test]
async fn same_link_values_after_unlink_do_not_revive_a_decision() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture().await;
    let choice = client
        .prepare_group_identity(&receiver, &user, &receiver_identity, local)
        .await
        .unwrap();
    let linked = db::user_db::get_user_by_id(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    db::user_db::clear_user_sync(&receiver.db, user.id)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(
        &receiver.db,
        user.id,
        linked.sync_token.as_deref().unwrap(),
        linked.sync_url.as_deref().unwrap(),
        false,
    )
    .await
    .unwrap();
    assert!(
        client
            .confirm_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
            .await
            .is_err()
    );
    let birth: Option<String> =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id=?")
            .bind(local)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    assert_eq!(birth, None);
}

#[tokio::test]
async fn identity_observation_and_name_choice_relay_to_original_device() {
    Box::pin(assert_identity_name_relay(false)).await;
}

#[tokio::test]
async fn populated_identity_name_relay_preserves_remote_child() {
    Box::pin(assert_identity_name_relay(true)).await;
}

#[allow(clippy::too_many_lines)] // Exercise original author and recipient through the complete relay.
async fn assert_identity_name_relay(populated: bool) {
    let Fixture {
        receiver,
        author,
        user,
        receiver_identity,
        author_identity,
        client,
        sender,
        local,
        remote,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture_with_remote_model(populated).await;
    let choice = client
        .prepare_group_identity(&receiver, &user, &receiver_identity, local)
        .await
        .unwrap();
    let linked = client
        .confirm_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
        .await
        .unwrap();
    client
        .publish_group_identity(&receiver, &user, &receiver_identity, &linked.event)
        .await
        .unwrap();
    sender
        .pull_next(&author, &user, &author_identity)
        .await
        .unwrap();
    sender
        .pull_next(&author, &user, &author_identity)
        .await
        .unwrap();
    let NameState::Conflict { choices, token } =
        db::causal_group_lifecycle::get_state(&author.db, &user, &author_identity, remote)
            .await
            .unwrap()
            .name
    else {
        panic!("the original device must see both observed names")
    };
    let mut names: Vec<_> = choices.iter().map(|choice| choice.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["local name", "remote name"]);
    assert!(
        matches!(db::causal_group_rename::resolve(&author.db, &user, &author_identity, remote, &token, "local name").await.unwrap(), ResolveResult::Resolved(NameState::Ready { ref name, .. }) if name == "local name")
    );
    let NameState::Ready { token, .. } =
        db::causal_group_lifecycle::get_state(&author.db, &user, &author_identity, remote)
            .await
            .unwrap()
            .name
    else {
        panic!("resolution must create an authored operation")
    };
    sender
        .publish_operation(&author, &user, &author_identity, &token.frontier[0])
        .await
        .unwrap();
    client
        .pull_next(&receiver, &user, &receiver_identity)
        .await
        .unwrap();
    client
        .pull_next(&receiver, &user, &receiver_identity)
        .await
        .unwrap();
    assert!(
        matches!(db::causal_group_lifecycle::get_state(&receiver.db, &user, &receiver_identity, local).await.unwrap().name, NameState::Ready { ref name, .. } if name == "local name")
    );
    for (state, group) in [(&author, remote), (&receiver, local)] {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM models WHERE model_user_id=? AND model_group_id=?",
        )
        .bind(user.id)
        .bind(group)
        .fetch_one(&*state.db)
        .await
        .unwrap();
        assert_eq!(count, i64::from(populated));
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // The test follows an identity choice across three independent devices.
async fn identity_linked_name_choice_becomes_stale_after_third_device_rename() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        global,
        token,
        dirs: _dirs,
        server,
        ..
    } = fixture().await;
    let choice = client
        .prepare_group_identity(&receiver, &user, &receiver_identity, local)
        .await
        .unwrap();
    client
        .confirm_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
        .await
        .unwrap();
    let NameState::Conflict { token: stale, .. } =
        db::causal_group_lifecycle::get_state(&receiver.db, &user, &receiver_identity, local)
            .await
            .unwrap()
            .name
    else {
        panic!("expected two independently observed names")
    };

    let (_third_dir, third) = crate::test_support::app_state().await;
    let third_identity = db::user_db::get_sync_identity(&third.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let third_client = AuthenticatedHub::pair(
        &third,
        &user,
        &third_identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    third_client
        .pull_next(&third, &user, &third_identity)
        .await
        .unwrap();
    let third_group: i64 = sqlx::query_scalar(
        "SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&*third.db)
    .await
    .unwrap();
    db::group_db::edit_group(&third.db, &user, third_group, "third name", None, None)
        .await
        .unwrap();
    let NameState::Ready { token, .. } =
        db::causal_group_lifecycle::get_state(&third.db, &user, &third_identity, third_group)
            .await
            .unwrap()
            .name
    else {
        panic!("third-device rename must be exportable")
    };
    third_client
        .publish_operation(&third, &user, &third_identity, &token.frontier[0])
        .await
        .unwrap();
    client
        .pull_next(&receiver, &user, &receiver_identity)
        .await
        .unwrap();
    assert!(
        matches!(
            db::causal_group_rename::resolve(
                &receiver.db,
                &user,
                &receiver_identity,
                local,
                &stale,
                "local name"
            )
            .await
            .unwrap(),
            ResolveResult::Stale(_)
        ),
        "an old decision cannot discard the third-device rename"
    );
    let NameState::Conflict { choices, .. } =
        db::causal_group_lifecycle::get_state(&receiver.db, &user, &receiver_identity, local)
            .await
            .unwrap()
            .name
    else {
        panic!("third-device change must remain in conflict")
    };
    let mut names: Vec<_> = choices.iter().map(|choice| choice.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["local name", "third name"]);
}

#[tokio::test]
async fn separate_legacy_groups_preserves_identity_history_and_retries() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        global,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture().await;
    let old_head: (i64, i64) = sqlx::query_as(
        "SELECT generation,head FROM group_sync_heads WHERE user_id=? AND group_global_id=?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    let choice = client
        .prepare_group_identity(&receiver, &user, &receiver_identity, local)
        .await
        .unwrap();
    let result = client
        .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
        .await
        .unwrap();
    assert!(!result.already_present);
    assert_eq!(result.local_group_id, local);
    assert_ne!(result.remote_group_id, local);
    let local_row: (String, String, Option<String>) = sqlx::query_as("SELECT group_name,group_unique_global_id,group_birth_id FROM models_group WHERE group_id=?").bind(local).fetch_one(&*receiver.db).await.unwrap();
    let remote_row: (String, String, Option<String>) = sqlx::query_as("SELECT group_name,group_unique_global_id,group_birth_id FROM models_group WHERE group_id=?").bind(result.remote_group_id).fetch_one(&*receiver.db).await.unwrap();
    assert_eq!(local_row.0, "local name");
    assert_ne!(local_row.1, global);
    assert!(local_row.2.is_some());
    assert_eq!(remote_row.0, "remote name");
    assert_eq!(remote_row.1, global);
    assert!(remote_row.2.is_some());
    assert_ne!(local_row.2, remote_row.2);
    let new_head: (i64, i64) = sqlx::query_as(
        "SELECT generation,head FROM group_sync_heads WHERE user_id=? AND group_global_id=?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    assert!(
        new_head.0 >= old_head.0 && new_head.1 > old_head.1,
        "historical head must advance, not reset"
    );
    let captured: String = sqlx::query_scalar(
        "SELECT original_head FROM group_identity_separations WHERE user_id=? AND decision_id=?",
    )
    .bind(user.id)
    .bind(&choice.decision_id)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    let captured: serde_json::Value = serde_json::from_str(&captured).unwrap();
    assert_eq!(captured["group_global_id"], global);
    assert_eq!(captured["generation"], old_head.0);
    assert_eq!(captured["head"], old_head.1);
    assert!(captured["last_deletion_operation_id"].is_null());
    assert!(sqlx::query("UPDATE group_identity_separations SET original_head='{}' WHERE user_id=? AND decision_id=?")
        .bind(user.id).bind(&choice.decision_id).execute(&*receiver.db).await.is_err(), "the captured original head is immutable");
    let wire: String = sqlx::query_scalar(
        "SELECT envelope FROM causal_events WHERE user_id=? AND writer_id=? AND counter=?",
    )
    .bind(user.id)
    .bind(&result.event.writer_id)
    .bind(result.event.counter.parse::<i64>().unwrap())
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    let event = parse_causal_envelope(&wire).unwrap();
    assert!(matches!(event.action, Action::LegacyObserved { .. }));
    assert!(
        event.parents.is_empty(),
        "separation must not invent creation history"
    );
    let retry = client
        .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
        .await
        .unwrap();
    assert!(retry.already_present);
    assert_eq!(
        (retry.local_group_id, retry.remote_group_id, retry.event),
        (result.local_group_id, result.remote_group_id, result.event)
    );
    assert!(
        client
            .confirm_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
            .await
            .is_err(),
        "the opposite choice cannot reuse a committed decision"
    );
}

#[tokio::test]
async fn separation_rejects_stale_or_foreign_decisions_without_changes() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        global,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture().await;
    let choice = client
        .prepare_group_identity(&receiver, &user, &receiver_identity, local)
        .await
        .unwrap();
    let other_id = db::user_db::add_user(
        &receiver.db,
        "other",
        "separate-other@example.com",
        &db::random_hex_32(),
    )
    .await
    .unwrap();
    let other = db::user_db::get_user_by_id(&receiver.db, other_id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = db::user_db::get_sync_identity(&receiver.db, other_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        client
            .separate_group_identity(&receiver, &other, &other_identity, &choice.decision_id)
            .await
            .is_err()
    );
    db::group_db::edit_group(&receiver.db, &user, local, "new name", None, None)
        .await
        .unwrap();
    db::group_db::edit_group(&receiver.db, &user, local, "local name", None, None)
        .await
        .unwrap();
    assert!(
        client
            .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
            .await
            .is_err(),
        "reverting a rename must not revive an old decision"
    );
    let row: (String, String, Option<String>) = sqlx::query_as("SELECT group_name,group_unique_global_id,group_birth_id FROM models_group WHERE group_id=?").bind(local).fetch_one(&*receiver.db).await.unwrap();
    assert_eq!(row, ("local name".into(), global, None));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models_group WHERE group_user_id=?")
            .bind(user.id)
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn separation_receipt_failure_rolls_back_both_groups_and_retries() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        global,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture().await;
    let choice = client
        .prepare_group_identity(&receiver, &user, &receiver_identity, local)
        .await
        .unwrap();
    let head: (i64, i64, String) = sqlx::query_as(
        "SELECT generation,head,state FROM group_sync_heads WHERE user_id=? AND group_global_id=?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER fail_separation_receipt BEFORE INSERT ON sync_pull_receipts BEGIN SELECT RAISE(ABORT,'injected failure'); END").execute(&*receiver.db).await.unwrap();
    assert!(
        client
            .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models_group WHERE group_user_id=?")
            .bind(user.id)
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        1
    );
    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT group_unique_global_id,group_birth_id FROM models_group WHERE group_id=?",
    )
    .bind(local)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    assert_eq!(row, (global.clone(), None));
    assert_eq!(sqlx::query_as::<_, (i64, i64, String)>("SELECT generation,head,state FROM group_sync_heads WHERE user_id=? AND group_global_id=?").bind(user.id).bind(&global).fetch_one(&*receiver.db).await.unwrap(), head);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM causal_events")
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER fail_separation_receipt")
        .execute(&*receiver.db)
        .await
        .unwrap();
    assert!(
        !client
            .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
            .await
            .unwrap()
            .already_present
    );
}

#[tokio::test]
async fn separated_groups_arrive_as_distinct_on_a_third_device() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        global,
        token,
        dirs: _dirs,
        server,
        ..
    } = fixture().await;
    let choice = client
        .prepare_group_identity(&receiver, &user, &receiver_identity, local)
        .await
        .unwrap();
    let result = client
        .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
        .await
        .unwrap();
    let queued = client
        .queue_observation(&receiver, &user, &receiver_identity, result.local_group_id)
        .await
        .unwrap();
    client
        .publish_queued(&receiver, &user, &receiver_identity, &queued)
        .await
        .unwrap();
    let (_third_dir, third) = crate::test_support::app_state().await;
    let third_identity = db::user_db::get_sync_identity(&third.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let third_client = AuthenticatedHub::pair(
        &third,
        &user,
        &third_identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    third_client
        .pull_next(&third, &user, &third_identity)
        .await
        .unwrap();
    third_client
        .pull_next(&third, &user, &third_identity)
        .await
        .unwrap();
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT group_name,group_unique_global_id FROM models_group WHERE group_user_id=? ORDER BY group_name").bind(user.id).fetch_all(&*third.db).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "local name");
    assert_ne!(rows[0].1, global);
    assert_eq!(rows[1], ("remote name".into(), global));
}

#[tokio::test]
async fn separation_rejects_prior_membership_history_even_after_group_is_empty() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        global,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture().await;
    let blob = db::blob_db::add_blob(&receiver.db, &db::random_hex_32(), "stl", 3, None)
        .await
        .unwrap();
    let model = db::model_db::add_model(&receiver.db, &user, "imported model", blob, None, None)
        .await
        .unwrap();
    sqlx::query("UPDATE models SET model_group_id=? WHERE model_id=?")
        .bind(local)
        .bind(model)
        .execute(&*receiver.db)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&receiver.db, &user, None, vec![model], None)
        .await
        .unwrap();
    let events_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM membership_causal_events WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    assert!(
        events_before > 0,
        "ungrouping the imported member must retain membership history"
    );
    let choice = client
        .prepare_group_identity(&receiver, &user, &receiver_identity, local)
        .await
        .unwrap();
    let head_before: (i64, i64, String) = sqlx::query_as(
        "SELECT generation,head,state FROM group_sync_heads WHERE user_id=? AND group_global_id=?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    assert!(
        client
            .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
            .await
            .is_err(),
        "prior membership history must prevent rekeying this group"
    );
    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT group_unique_global_id,group_birth_id FROM models_group WHERE group_id=?",
    )
    .bind(local)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    assert_eq!(row, (global.clone(), None));
    let head_after: (i64, i64, String) = sqlx::query_as(
        "SELECT generation,head,state FROM group_sync_heads WHERE user_id=? AND group_global_id=?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    assert_eq!(head_before, head_after);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM membership_causal_events WHERE user_id=?"
        )
        .bind(user.id)
        .fetch_one(&*receiver.db)
        .await
        .unwrap(),
        events_before
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn separation_rejects_retained_portable_references_to_old_group() {
    for field in ["from", "to", "group", "dependency"] {
        let Fixture {
            receiver,
            user,
            receiver_identity,
            client,
            local,
            global,
            dirs: _dirs,
            server: _server,
            ..
        } = fixture().await;
        let group =
            serde_json::json!({"kind":"group","global_id":global,"birth_id":db::random_hex_32()});
        let model = serde_json::json!({"kind":"model","global_id":db::random_hex_32(),"birth_id":db::random_hex_32()});
        let event = serde_json::json!({"writer_id":db::random_hex_32(),"counter":"1"});
        let mut envelope = serde_json::json!({"version":2,"entity":model,"event":event,"parents":[],"dependencies":[],"action":{"type":"model_moved","from":null,"to":null,"target_lifecycle":null}});
        match field {
            "from" | "to" => envelope["action"][field] = group,
            "group" => {
                envelope["action"] = serde_json::json!({"type":"model_detached","group":group,"deletion":{"writer_id":db::random_hex_32(),"counter":"1"}});
            }
            "dependency" => {
                envelope["dependencies"] = serde_json::json!([{"entity":group,"event":{"writer_id":db::random_hex_32(),"counter":"1"}}]);
            }
            _ => unreachable!(),
        }
        let writer = envelope["event"]["writer_id"].as_str().unwrap();
        sqlx::query("INSERT INTO causal_events(user_id,writer_id,counter,envelope,encoding) VALUES(?,?,1,?,'portable_v2')")
            .bind(user.id).bind(writer).bind(envelope.to_string()).execute(&*receiver.db).await.unwrap();
        let choice = client
            .prepare_group_identity(&receiver, &user, &receiver_identity, local)
            .await
            .unwrap();
        assert!(
            client
                .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
                .await
                .is_err(),
            "retained portable {field} reference must prevent rekeying"
        );
        let row: (String, Option<String>) = sqlx::query_as(
            "SELECT group_unique_global_id,group_birth_id FROM models_group WHERE group_id=?",
        )
        .bind(local)
        .fetch_one(&*receiver.db)
        .await
        .unwrap();
        assert_eq!(row, (global, None));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM causal_events WHERE user_id=?")
                .bind(user.id)
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn separation_fails_closed_with_legacy_assignment_journal() {
    for kind in ["valid", "valid_unrelated", "malformed", "non_utf8"] {
        let Fixture {
            receiver,
            user,
            receiver_identity,
            client,
            local,
            global,
            dirs: _dirs,
            server: _server,
            ..
        } = fixture().await;
        let payload = match kind {
            "valid" => {
                format!("lifecycle-v1;g{}:{global}1:11:16:active1:0-", global.len()).into_bytes()
            }
            "valid_unrelated" => {
                let other_global = db::random_hex_32();
                format!(
                    "lifecycle-v1;g{}:{other_global}1:11:16:active1:0-",
                    other_global.len()
                )
                .into_bytes()
            }
            "malformed" => b"not a recognized journal encoding".to_vec(),
            "non_utf8" => vec![0xff, 0xfe, 0x00],
            _ => unreachable!(),
        };
        sqlx::query(
            "INSERT INTO assignment_batch_journal(user_id,operation_id,payload) VALUES(?,?,?)",
        )
        .bind(user.id)
        .bind(db::random_hex_32())
        .bind(&payload)
        .execute(&*receiver.db)
        .await
        .unwrap();
        let choice = client
            .prepare_group_identity(&receiver, &user, &receiver_identity, local)
            .await
            .unwrap();
        assert!(
            client
                .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
                .await
                .is_err(),
            "legacy {kind} journal cannot be silently discarded"
        );
        let row: (String, Option<String>) = sqlx::query_as(
            "SELECT group_unique_global_id,group_birth_id FROM models_group WHERE group_id=?",
        )
        .bind(local)
        .fetch_one(&*receiver.db)
        .await
        .unwrap();
        assert_eq!(row, (global, None));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM assignment_batch_journal WHERE user_id=?"
            )
            .bind(user.id)
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Exercise both user choices and their idempotent retry against one source model.
async fn populated_remote_identity_choice_preserves_child_identity_and_bytes() {
    for same_group in [true, false] {
        let Fixture {
            receiver,
            user,
            receiver_identity,
            client,
            local,
            child,
            dirs: _dirs,
            server: _server,
            ..
        } = fixture_with_remote_model(true).await;
        let (child_global, child_birth) = child.unwrap();
        let choice = client
            .prepare_group_identity(&receiver, &user, &receiver_identity, local)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models WHERE model_user_id=?")
                .bind(user.id)
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            0,
            "preparing a choice cannot install a remote child"
        );
        let target = if same_group {
            client
                .confirm_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
                .await
                .unwrap()
                .group_id
        } else {
            client
                .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
                .await
                .unwrap()
                .remote_group_id
        };
        assert_eq!(target == local, same_group);
        if same_group {
            assert!(
                client
                    .confirm_group_identity(
                        &receiver,
                        &user,
                        &receiver_identity,
                        &choice.decision_id
                    )
                    .await
                    .unwrap()
                    .already_present
            );
            assert!(
                client
                    .separate_group_identity(
                        &receiver,
                        &user,
                        &receiver_identity,
                        &choice.decision_id
                    )
                    .await
                    .is_err()
            );
        } else {
            assert!(
                client
                    .separate_group_identity(
                        &receiver,
                        &user,
                        &receiver_identity,
                        &choice.decision_id
                    )
                    .await
                    .unwrap()
                    .already_present
            );
            assert!(
                client
                    .confirm_group_identity(
                        &receiver,
                        &user,
                        &receiver_identity,
                        &choice.decision_id
                    )
                    .await
                    .is_err()
            );
        }
        let (group, birth, path): (i64, String, String) = sqlx::query_as("SELECT m.model_group_id,m.model_birth_id,b.blob_path FROM models m JOIN blobs b ON b.blob_id=m.model_blob_id WHERE m.model_user_id=? AND m.model_unique_global_id=?").bind(user.id).bind(&child_global).fetch_one(&*receiver.db).await.unwrap();
        assert_eq!(group, target);
        assert_eq!(birth, child_birth, "the remote child retains its identity");
        assert_eq!(std::fs::read(path).unwrap(), b"abc");
        if same_group {
            assert!(
                matches!(
                    db::causal_group_lifecycle::get_state(
                        &receiver.db,
                        &user,
                        &receiver_identity,
                        local
                    )
                    .await
                    .unwrap()
                    .name,
                    db::causal_group_rename::NameState::Conflict { .. }
                ),
                "both group names require a choice"
            );
        } else {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM models_group WHERE group_user_id=?"
                )
                .bind(user.id)
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
                2
            );
        }
    }
}

#[tokio::test]
async fn populated_remote_child_collision_never_adopts_or_overwrites_local_model() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        child,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture_with_remote_model(true).await;
    let (child_global, _) = child.unwrap();
    let blob = db::blob_db::add_blob(
        &receiver.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        None,
    )
    .await
    .unwrap();
    let existing = db::model_db::add_model(&receiver.db, &user, "existing local", blob, None, None)
        .await
        .unwrap();
    sqlx::query("UPDATE models SET model_unique_global_id=? WHERE model_id=?")
        .bind(&child_global)
        .bind(existing)
        .execute(&*receiver.db)
        .await
        .unwrap();
    // Preparation must refuse for exactly this reason; any other error means the
    // collision path was never reached.
    let refusal = client
        .prepare_group_identity(&receiver, &user, &receiver_identity, local)
        .await
        .expect_err("an occupied child identity must be refused at preparation");
    assert!(
        matches!(
            refusal,
            service::ServiceError::DatabaseError(db::DbError::Protocol(ref reason))
                if reason == "Invalid or unsupported pristine genesis"
        ),
        "a locally occupied child identity must never be silently matched"
    );
    let row: (String, Option<i64>) =
        sqlx::query_as("SELECT model_name,model_group_id FROM models WHERE model_id=?")
            .bind(existing)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    assert_eq!(row, ("existing local".into(), None));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn populated_choice_rejects_legacy_scalar_child_history() {
    let Fixture {
        receiver,
        user,
        receiver_identity,
        client,
        local,
        child,
        dirs: _dirs,
        server: _server,
        ..
    } = fixture_with_remote_model(true).await;
    let (child_global, _) = child.unwrap();
    let history = serde_json::json!({"entity":{"kind":"group","global_id":db::random_hex_32(),"birth_id":null,"local_incarnation":null},"ancestry":"unknown","action":{"kind":"member_added","model_global_id":child_global,"model_incarnation":db::random_hex_32()}});
    sqlx::query("INSERT INTO causal_events(user_id,writer_id,counter,envelope,encoding) VALUES(?,?,1,?,'local_history_v1')").bind(user.id).bind(db::random_hex_32()).bind(history.to_string()).execute(&*receiver.db).await.unwrap();
    assert!(
        client
            .prepare_group_identity(&receiver, &user, &receiver_identity, local)
            .await
            .is_err(),
        "retained legacy membership evidence prevents assuming the incoming child is new"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models WHERE model_user_id=?")
            .bind(user.id)
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn populated_choice_rechecks_local_state_after_blob_download() {
    for same_group in [true, false] {
        let reached = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let Fixture {
            receiver,
            user,
            receiver_identity,
            client,
            local,
            dirs: _dirs,
            server: _server,
            ..
        } = fixture_with_download_barrier(true, Some((reached.clone(), release.clone())), false)
            .await;
        let choice = client
            .prepare_group_identity(&receiver, &user, &receiver_identity, local)
            .await
            .unwrap();
        let decide = async {
            if same_group {
                client
                    .confirm_group_identity(
                        &receiver,
                        &user,
                        &receiver_identity,
                        &choice.decision_id,
                    )
                    .await
                    .is_err()
            } else {
                client
                    .separate_group_identity(
                        &receiver,
                        &user,
                        &receiver_identity,
                        &choice.decision_id,
                    )
                    .await
                    .is_err()
            }
        };
        let change = async {
            reached.notified().await;
            db::group_db::edit_group(&receiver.db, &user, local, "changed", None, None)
                .await
                .unwrap();
            db::group_db::edit_group(&receiver.db, &user, local, "local name", None, None)
                .await
                .unwrap();
            release.notify_one();
        };
        let (rejected, ()) = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            Box::pin(async { tokio::join!(decide, change) }),
        )
        .await
        .expect("blob download must reach the barrier");
        assert!(
            rejected,
            "stale decision must not overwrite edits made during download"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models WHERE model_user_id=?")
                .bind(user.id)
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn populated_choice_rechecks_child_identity_after_download() {
    for same_group in [true, false] {
        let reached = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let Fixture {
            receiver,
            user,
            receiver_identity,
            client,
            local,
            child,
            dirs: _dirs,
            server: _server,
            ..
        } = fixture_with_download_barrier(true, Some((reached.clone(), release.clone())), false)
            .await;
        let (child_global, _) = child.unwrap();
        let choice = client
            .prepare_group_identity(&receiver, &user, &receiver_identity, local)
            .await
            .unwrap();
        let decide = async {
            if same_group {
                client
                    .confirm_group_identity(
                        &receiver,
                        &user,
                        &receiver_identity,
                        &choice.decision_id,
                    )
                    .await
                    .is_err()
            } else {
                client
                    .separate_group_identity(
                        &receiver,
                        &user,
                        &receiver_identity,
                        &choice.decision_id,
                    )
                    .await
                    .is_err()
            }
        };
        let collide = async {
            reached.notified().await;
            let blob = db::blob_db::add_blob(
                &receiver.db,
                "ba7816bf8f01cfea414140de5dae2223",
                "stl",
                3,
                None,
            )
            .await
            .unwrap();
            let existing =
                db::model_db::add_model(&receiver.db, &user, "new local child", blob, None, None)
                    .await
                    .unwrap();
            sqlx::query("UPDATE models SET model_unique_global_id=? WHERE model_id=?")
                .bind(&child_global)
                .bind(existing)
                .execute(&*receiver.db)
                .await
                .unwrap();
            release.notify_one();
        };
        let (rejected, ()) = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            Box::pin(async { tokio::join!(decide, collide) }),
        )
        .await
        .expect("blob download must reach the barrier");
        assert!(
            rejected,
            "a child identity occupied during download cannot be adopted"
        );
        let row: (String, Option<i64>) = sqlx::query_as("SELECT model_name,model_group_id FROM models WHERE model_user_id=? AND model_unique_global_id=?").bind(user.id).bind(&child_global).fetch_one(&*receiver.db).await.unwrap();
        assert_eq!(row, ("new local child".into(), None));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn populated_choice_receipt_failure_rolls_back_children_and_retries() {
    for same_group in [true, false] {
        let Fixture {
            receiver,
            user,
            receiver_identity,
            client,
            local,
            dirs: _dirs,
            server: _server,
            ..
        } = fixture_with_remote_model(true).await;
        let choice = client
            .prepare_group_identity(&receiver, &user, &receiver_identity, local)
            .await
            .unwrap();
        sqlx::query("CREATE TRIGGER fail_populated_choice_receipt BEFORE INSERT ON sync_pull_receipts BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END").execute(&*receiver.db).await.unwrap();
        let rejected = if same_group {
            client
                .confirm_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
                .await
                .is_err()
        } else {
            client
                .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
                .await
                .is_err()
        };
        assert!(rejected);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models WHERE model_user_id=?")
                .bind(user.id)
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models_group WHERE group_user_id=?")
                .bind(user.id)
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            0
        );
        sqlx::query("DROP TRIGGER fail_populated_choice_receipt")
            .execute(&*receiver.db)
            .await
            .unwrap();
        if same_group {
            assert!(
                !client
                    .confirm_group_identity(
                        &receiver,
                        &user,
                        &receiver_identity,
                        &choice.decision_id
                    )
                    .await
                    .unwrap()
                    .already_present
            );
        } else {
            assert!(
                !client
                    .separate_group_identity(
                        &receiver,
                        &user,
                        &receiver_identity,
                        &choice.decision_id
                    )
                    .await
                    .unwrap()
                    .already_present
            );
        }
    }
}

#[tokio::test]
async fn populated_choice_rejects_corrupted_remote_file_without_installing() {
    for same_group in [true, false] {
        let Fixture {
            receiver,
            user,
            receiver_identity,
            client,
            local,
            dirs: _dirs,
            server: _server,
            ..
        } = fixture_with_download_barrier(true, None, true).await;
        let choice = client
            .prepare_group_identity(&receiver, &user, &receiver_identity, local)
            .await
            .unwrap();
        let rejected = if same_group {
            client
                .confirm_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
                .await
                .is_err()
        } else {
            client
                .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
                .await
                .is_err()
        };
        assert!(rejected, "file bytes must match the authenticated digest");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models WHERE model_user_id=?")
                .bind(user.id)
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models_group WHERE group_user_id=?")
                .bind(user.id)
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Both remote shapes must preserve the local model and publish its staged bytes after source changes.
async fn keep_both_populated_local_preserves_data_and_publishes_original_bytes() {
    for remote_populated in [false, true] {
        let Fixture {
            receiver,
            user,
            receiver_identity,
            client,
            local,
            global,
            token,
            child,
            dirs,
            server,
            ..
        } = fixture_with_remote_model(remote_populated).await;
        let local_path = dirs[2].path().join("local-original.stl");
        std::fs::write(&local_path, b"xyz").unwrap();
        let blob = db::blob_db::add_blob(
            &receiver.db,
            "3608bca1e44ea6c4d268eb6db0226026",
            "stl",
            3,
            Some(local_path.to_string_lossy().into_owned()),
        )
        .await
        .unwrap();
        let model = db::model_db::add_model(&receiver.db, &user, "local model", blob, None, None)
            .await
            .unwrap();
        sqlx::query("UPDATE models SET model_group_id=? WHERE model_id=?")
            .bind(local)
            .bind(model)
            .execute(&*receiver.db)
            .await
            .unwrap();
        let before: (String, Option<String>, i64) = sqlx::query_as("SELECT model_unique_global_id,model_birth_id,model_blob_id FROM models WHERE model_id=?").bind(model).fetch_one(&*receiver.db).await.unwrap();
        let choice = client
            .prepare_group_identity(&receiver, &user, &receiver_identity, local)
            .await
            .unwrap();
        assert!(
            choice.same_group_supported,
            "disjoint children allow either user choice"
        );
        let result = client
            .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
            .await
            .unwrap();
        assert_eq!(result.local_group_id, local);
        let outbox = result
            .outbox_id
            .clone()
            .expect("local populated group needs a durable upload");
        let after: (String, Option<String>, i64, i64) = sqlx::query_as("SELECT model_unique_global_id,model_birth_id,model_blob_id,model_group_id FROM models WHERE model_id=?").bind(model).fetch_one(&*receiver.db).await.unwrap();
        assert_eq!(after, (before.0.clone(), before.1.clone(), before.2, local));
        assert_eq!(std::fs::read(&local_path).unwrap(), b"xyz");
        let retry = client
            .separate_group_identity(&receiver, &user, &receiver_identity, &choice.decision_id)
            .await
            .unwrap();
        assert!(retry.already_present);
        assert_eq!(retry.outbox_id.as_deref(), Some(outbox.as_str()));
        if let Some((remote_global, remote_birth)) = child {
            let row: (i64, String, String) = sqlx::query_as("SELECT m.model_group_id,m.model_birth_id,b.blob_path FROM models m JOIN blobs b ON b.blob_id=m.model_blob_id WHERE m.model_user_id=? AND m.model_unique_global_id=?").bind(user.id).bind(remote_global).fetch_one(&*receiver.db).await.unwrap();
            assert_eq!(row.0, result.remote_group_id);
            assert_eq!(row.1, remote_birth);
            assert_eq!(std::fs::read(row.2).unwrap(), b"abc");
        }
        std::fs::write(&local_path, b"BAD").unwrap();
        client
            .publish_queued(&receiver, &user, &receiver_identity, &outbox)
            .await
            .unwrap();
        let (_third_dir, third) = crate::test_support::app_state().await;
        let third_identity = db::user_db::get_sync_identity(&third.db, user.id)
            .await
            .unwrap()
            .unwrap();
        let third_client = AuthenticatedHub::pair(
            &third,
            &user,
            &third_identity,
            &server.origin(),
            &token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
        )
        .await
        .unwrap();
        third_client
            .pull_next(&third, &user, &third_identity)
            .await
            .unwrap();
        third_client
            .pull_next(&third, &user, &third_identity)
            .await
            .unwrap();
        let (path, birth): (String, String) = sqlx::query_as("SELECT b.blob_path,m.model_birth_id FROM models m JOIN blobs b ON b.blob_id=m.model_blob_id WHERE m.model_user_id=? AND m.model_unique_global_id=?").bind(user.id).bind(&before.0).fetch_one(&*third.db).await.unwrap();
        assert_eq!(
            std::fs::read(path).unwrap(),
            b"xyz",
            "published bytes are the snapshot, not later source edits"
        );
        assert_eq!(Some(birth), before.1);
        let remote_id: String =
            sqlx::query_scalar("SELECT group_unique_global_id FROM models_group WHERE group_id=?")
                .bind(result.remote_group_id)
                .fetch_one(&*receiver.db)
                .await
                .unwrap();
        assert_eq!(remote_id, global);
    }
}

#[tokio::test]
async fn keep_both_populated_receipt_failure_rolls_back_both_groups_and_outbox() {
    let fixture = fixture_with_remote_model(true).await;
    let (model, path) = add_imported_local_model(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_populated_separation BEFORE INSERT ON sync_pull_receipts BEGIN SELECT RAISE(ABORT,'injected failure'); END").execute(&*fixture.receiver.db).await.unwrap();
    assert!(
        fixture
            .client
            .separate_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id
            )
            .await
            .is_err()
    );
    let group: (String, Option<String>) = sqlx::query_as(
        "SELECT group_unique_global_id,group_birth_id FROM models_group WHERE group_id=?",
    )
    .bind(fixture.local)
    .fetch_one(&*fixture.receiver.db)
    .await
    .unwrap();
    assert_eq!(group, (fixture.global.clone(), None));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(model)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        fixture.local
    );
    assert_eq!(std::fs::read(path).unwrap(), b"xyz");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models WHERE model_user_id=?")
            .bind(fixture.user.id)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models_group WHERE group_user_id=?")
            .bind(fixture.user.id)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_publication_outbox")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER fail_populated_separation")
        .execute(&*fixture.receiver.db)
        .await
        .unwrap();
    assert!(
        fixture
            .client
            .separate_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id
            )
            .await
            .unwrap()
            .outbox_id
            .is_some()
    );
}

#[tokio::test]
async fn keep_both_populated_rechecks_local_model_after_remote_download() {
    let reached = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let fixture =
        fixture_with_download_barrier(true, Some((reached.clone(), release.clone())), false).await;
    let (model, path) = add_imported_local_model(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    let decide = fixture.client.separate_group_identity(
        &fixture.receiver,
        &fixture.user,
        &fixture.receiver_identity,
        &choice.decision_id,
    );
    let edit = async {
        reached.notified().await;
        sqlx::query("UPDATE models SET model_name='changed' WHERE model_id=?")
            .bind(model)
            .execute(&*fixture.receiver.db)
            .await
            .unwrap();
        sqlx::query("UPDATE models SET model_name='local model' WHERE model_id=?")
            .bind(model)
            .execute(&*fixture.receiver.db)
            .await
            .unwrap();
        release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        Box::pin(async { tokio::join!(decide, edit) }),
    )
    .await
    .expect("download must reach the barrier");
    assert!(
        result.is_err(),
        "model metadata ABA invalidates the prepared separation"
    );
    assert_eq!(std::fs::read(path).unwrap(), b"xyz");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models WHERE model_user_id=?")
            .bind(fixture.user.id)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_publication_outbox")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Two independent populated observations and their files must survive relay to two devices.
async fn same_populated_groups_keep_disjoint_children_and_relay_independent_roots() {
    let fixture = fixture_with_remote_model(true).await;
    let (local_model, local_path) = add_imported_local_model(&fixture).await;
    let (local_global, local_birth, local_blob): (String, String, i64) = sqlx::query_as(
        "SELECT model_unique_global_id,model_birth_id,model_blob_id FROM models WHERE model_id=?",
    )
    .bind(local_model)
    .fetch_one(&*fixture.receiver.db)
    .await
    .unwrap();
    let (remote_global, remote_birth) = fixture.child.clone().unwrap();
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    assert!(choice.same_group_supported);
    let result = fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap();
    assert_eq!(result.group_id, fixture.local);
    let outbox = result
        .outbox_id
        .clone()
        .expect("local populated observation must be durably queued");
    let local_row: (String, String, i64, i64) = sqlx::query_as("SELECT model_unique_global_id,model_birth_id,model_blob_id,model_group_id FROM models WHERE model_id=?").bind(local_model).fetch_one(&*fixture.receiver.db).await.unwrap();
    assert_eq!(
        local_row,
        (
            local_global.clone(),
            local_birth.clone(),
            local_blob,
            fixture.local
        )
    );
    let (remote_group, actual_birth, remote_path): (i64, String, String) = sqlx::query_as("SELECT m.model_group_id,m.model_birth_id,b.blob_path FROM models m JOIN blobs b ON b.blob_id=m.model_blob_id WHERE m.model_user_id=? AND m.model_unique_global_id=?").bind(fixture.user.id).bind(&remote_global).fetch_one(&*fixture.receiver.db).await.unwrap();
    assert_eq!((remote_group, actual_birth), (fixture.local, remote_birth));
    assert_eq!(std::fs::read(remote_path).unwrap(), b"abc");
    let wire: String = sqlx::query_scalar(
        "SELECT envelope FROM causal_events WHERE user_id=? AND writer_id=? AND counter=?",
    )
    .bind(fixture.user.id)
    .bind(&result.event.writer_id)
    .bind(result.event.counter.parse::<i64>().unwrap())
    .fetch_one(&*fixture.receiver.db)
    .await
    .unwrap();
    let root = parse_causal_envelope(&wire).unwrap();
    assert!(
        root.parents.is_empty(),
        "the local observation must remain independent"
    );
    assert_eq!(root.dependencies.len(), 1);
    assert_eq!(root.dependencies[0].entity.global_id, local_global);
    assert_eq!(
        root.dependencies[0].entity.birth_id.as_deref(),
        Some(local_birth.as_str())
    );
    let retry = fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap();
    assert!(retry.already_present);
    assert_eq!(retry.outbox_id.as_deref(), Some(outbox.as_str()));
    std::fs::write(&local_path, b"BAD").unwrap();
    fixture
        .client
        .publish_queued(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &outbox,
        )
        .await
        .unwrap();
    fixture
        .sender
        .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap();
    fixture
        .sender
        .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap();
    let (_third_dir, third) = crate::test_support::app_state().await;
    let third_identity = db::user_db::get_sync_identity(&third.db, fixture.user.id)
        .await
        .unwrap()
        .unwrap();
    let third_client = AuthenticatedHub::pair(
        &third,
        &fixture.user,
        &third_identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    third_client
        .pull_next(&third, &fixture.user, &third_identity)
        .await
        .unwrap();
    third_client
        .pull_next(&third, &fixture.user, &third_identity)
        .await
        .unwrap();
    for state in [&fixture.author, &third] {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT m.model_unique_global_id,b.blob_path FROM models m JOIN blobs b ON b.blob_id=m.model_blob_id WHERE m.model_user_id=? ORDER BY m.model_unique_global_id").bind(fixture.user.id).fetch_all(&*state.db).await.unwrap();
        assert_eq!(
            rows.len(),
            2,
            "neither batch's absence deletes the other batch's child"
        );
        let path = rows
            .iter()
            .find(|(global, _)| global == &local_global)
            .unwrap()
            .1
            .as_str();
        assert_eq!(std::fs::read(path).unwrap(), b"xyz");
        assert!(rows.iter().any(|(global, _)| global == &remote_global));
    }
}

#[tokio::test]
async fn same_populated_groups_reject_overlapping_child_identity() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, path) = add_imported_local_model(&fixture).await;
    let (remote_global, _) = fixture.child.clone().unwrap();
    sqlx::query("UPDATE models SET model_unique_global_id=? WHERE model_id=?")
        .bind(&remote_global)
        .bind(local)
        .execute(&*fixture.receiver.db)
        .await
        .unwrap();
    let rejected = match fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
    {
        Err(_) => true,
        Ok(choice) if !choice.same_group_supported => true,
        Ok(choice) => fixture
            .client
            .confirm_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
            )
            .await
            .is_err(),
    };
    assert!(
        rejected,
        "overlapping children cannot be silently treated as the same model"
    );
    assert_eq!(std::fs::read(path).unwrap(), b"xyz");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models WHERE model_user_id=?")
            .bind(fixture.user.id)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn same_populated_group_receipt_failure_rolls_back_all_and_retries() {
    let fixture = fixture_with_remote_model(true).await;
    let (model, path) = add_imported_local_model(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_populated_match BEFORE INSERT ON sync_pull_receipts BEGIN SELECT RAISE(ABORT,'injected failure'); END").execute(&*fixture.receiver.db).await.unwrap();
    assert!(
        fixture
            .client
            .confirm_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id
            )
            .await
            .is_err()
    );
    let group: (String, Option<String>) = sqlx::query_as(
        "SELECT group_unique_global_id,group_birth_id FROM models_group WHERE group_id=?",
    )
    .bind(fixture.local)
    .fetch_one(&*fixture.receiver.db)
    .await
    .unwrap();
    assert_eq!(group, (fixture.global.clone(), None));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(model)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        fixture.local
    );
    assert_eq!(std::fs::read(path).unwrap(), b"xyz");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models WHERE model_user_id=?")
            .bind(fixture.user.id)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_publication_outbox")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER fail_populated_match")
        .execute(&*fixture.receiver.db)
        .await
        .unwrap();
    assert!(
        fixture
            .client
            .confirm_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id
            )
            .await
            .unwrap()
            .outbox_id
            .is_some()
    );
}

#[tokio::test]
async fn same_populated_group_rechecks_local_child_after_download() {
    let reached = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let fixture =
        fixture_with_download_barrier(true, Some((reached.clone(), release.clone())), false).await;
    let (model, path) = add_imported_local_model(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    let decide = fixture.client.confirm_group_identity(
        &fixture.receiver,
        &fixture.user,
        &fixture.receiver_identity,
        &choice.decision_id,
    );
    let edit = async {
        reached.notified().await;
        sqlx::query("UPDATE models SET model_name='changed' WHERE model_id=?")
            .bind(model)
            .execute(&*fixture.receiver.db)
            .await
            .unwrap();
        sqlx::query("UPDATE models SET model_name='local model' WHERE model_id=?")
            .bind(model)
            .execute(&*fixture.receiver.db)
            .await
            .unwrap();
        release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        Box::pin(async { tokio::join!(decide, edit) }),
    )
    .await
    .expect("download must reach barrier");
    assert!(
        result.is_err(),
        "local child metadata ABA invalidates the prepared decision"
    );
    assert_eq!(std::fs::read(path).unwrap(), b"xyz");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models WHERE model_user_id=?")
            .bind(fixture.user.id)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_publication_outbox")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Exercise independent rename and supplemental model through the relay.
async fn same_populated_supplement_does_not_overwrite_sender_later_rename() {
    let fixture = fixture_with_remote_model(true).await;
    let (local_model, _) = add_imported_local_model(&fixture).await;
    let local_global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id=?")
            .bind(local_model)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    let result = fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap();
    let outbox = result.outbox_id.unwrap();
    db::group_db::edit_group(
        &fixture.author.db,
        &fixture.user,
        fixture.remote,
        "later remote",
        None,
        None,
    )
    .await
    .unwrap();
    let NameState::Ready { token, .. } = db::causal_group_lifecycle::get_state(
        &fixture.author.db,
        &fixture.user,
        &fixture.author_identity,
        fixture.remote,
    )
    .await
    .unwrap()
    .name
    else {
        panic!("sender rename must be available")
    };
    fixture
        .sender
        .publish_operation(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            &token.frontier[0],
        )
        .await
        .unwrap();
    fixture
        .client
        .publish_queued(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &outbox,
        )
        .await
        .unwrap();
    for _ in 0..3 {
        fixture
            .sender
            .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
            .await
            .unwrap();
    }
    let NameState::Conflict { choices, .. } = db::causal_group_lifecycle::get_state(
        &fixture.author.db,
        &fixture.user,
        &fixture.author_identity,
        fixture.remote,
    )
    .await
    .unwrap()
    .name
    else {
        panic!("concurrent names must remain in conflict")
    };
    let mut names: Vec<_> = choices.iter().map(|choice| choice.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["later remote", "local name"]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM models WHERE model_user_id=? AND model_group_id=?"
        )
        .bind(fixture.user.id)
        .bind(fixture.remote)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM models WHERE model_user_id=? AND model_unique_global_id=?"
        )
        .bind(fixture.user.id)
        .bind(local_global)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Exercise an independent ungroup and supplemental model through the relay.
async fn same_populated_supplement_does_not_undo_sender_later_ungroup() {
    let fixture = fixture_with_remote_model(true).await;
    let (local_model, _) = add_imported_local_model(&fixture).await;
    let local_global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id=?")
            .bind(local_model)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    let remote_model: i64 = sqlx::query_scalar(
        "SELECT model_id FROM models WHERE model_user_id=? AND model_group_id=?",
    )
    .bind(fixture.user.id)
    .bind(fixture.remote)
    .fetch_one(&*fixture.author.db)
    .await
    .unwrap();
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    let result = fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap();
    let outbox = result.outbox_id.unwrap();
    db::group_db::set_group_id_on_models(
        &fixture.author.db,
        &fixture.user,
        None,
        vec![remote_model],
        None,
    )
    .await
    .unwrap();
    let MoveState::Ready { token, .. } = db::causal_model_move::get_state(
        &fixture.author.db,
        &fixture.user,
        &fixture.author_identity,
        remote_model,
    )
    .await
    .unwrap() else {
        panic!("sender move must be available")
    };
    fixture
        .sender
        .publish_operation(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            &token.frontier[0],
        )
        .await
        .unwrap();
    fixture
        .client
        .publish_queued(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &outbox,
        )
        .await
        .unwrap();
    for _ in 0..3 {
        fixture
            .sender
            .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
            .await
            .unwrap();
    }
    let group: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(remote_model)
            .fetch_one(&*fixture.author.db)
            .await
            .unwrap();
    assert_eq!(
        group, None,
        "the sender's later move cannot be overwritten by the supplemental baseline"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT model_group_id FROM models WHERE model_user_id=? AND model_unique_global_id=?"
        )
        .bind(fixture.user.id)
        .bind(local_global)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        fixture.remote
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Both delivery orders must preserve the group, models and deletion evidence.
async fn concurrent_delete_and_populated_observation_keep_group_in_both_orders() {
    for delete_first in [true, false] {
        let fixture = fixture_with_remote_model(true).await;
        let (local_model, _) = add_imported_local_model(&fixture).await;
        let local_global: String =
            sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id=?")
                .bind(local_model)
                .fetch_one(&*fixture.receiver.db)
                .await
                .unwrap();
        let (remote_global, _) = fixture.child.clone().unwrap();
        let choice = fixture
            .client
            .prepare_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                fixture.local,
            )
            .await
            .unwrap();
        let result = fixture
            .client
            .confirm_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
            )
            .await
            .unwrap();
        let outbox = result.outbox_id.unwrap();
        db::group_db::delete_group(&fixture.author.db, &fixture.user, fixture.remote)
            .await
            .unwrap();
        let GroupLifecycle::Deleted { deletions } = db::causal_group_lifecycle::get_state(
            &fixture.author.db,
            &fixture.user,
            &fixture.author_identity,
            fixture.remote,
        )
        .await
        .unwrap()
        .lifecycle
        else {
            panic!("expected delete event")
        };
        let delete = || {
            fixture.sender.publish_operation(
                &fixture.author,
                &fixture.user,
                &fixture.author_identity,
                &deletions[0],
            )
        };
        let supplement = || {
            fixture.client.publish_queued(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &outbox,
            )
        };
        if delete_first {
            delete().await.unwrap();
            supplement().await.unwrap();
        } else {
            supplement().await.unwrap();
            delete().await.unwrap();
        }
        for _ in 0..3 {
            fixture
                .sender
                .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
                .await
                .unwrap();
        }
        for _ in 0..2 {
            fixture
                .client
                .pull_next(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
                .await
                .unwrap();
        }
        for state in [&fixture.author, &fixture.receiver] {
            let (group, deleted): (i64, Option<String>) = sqlx::query_as("SELECT group_id,group_deleted_at FROM models_group WHERE group_user_id=? AND group_unique_global_id=?").bind(fixture.user.id).bind(&fixture.global).fetch_one(&*state.db).await.unwrap();
            assert_eq!(
                deleted, None,
                "concurrent observed addition keeps the group active"
            );
            let added: Option<i64> = sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_user_id=? AND model_unique_global_id=?").bind(fixture.user.id).bind(&local_global).fetch_one(&*state.db).await.unwrap();
            assert_eq!(added, Some(group));
            let original: Option<i64> = sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_user_id=? AND model_unique_global_id=?").bind(fixture.user.id).bind(&remote_global).fetch_one(&*state.db).await.unwrap();
            assert_eq!(
                original,
                Some(group),
                "a losing deletion cannot ungroup an otherwise unchanged member"
            );
            assert!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM group_deletion_records WHERE user_id=? AND group_global_id=?").bind(fixture.user.id).bind(&fixture.global).fetch_one(&*state.db).await.unwrap() > 0);
        }
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Verify retention, identity-reuse rejection and late rehydration together.
async fn late_populated_observation_rehydrates_deleted_group_after_retention() {
    let fixture = fixture_with_remote_model(true).await;
    let (local_model, _) = add_imported_local_model(&fixture).await;
    let local_global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id=?")
            .bind(local_model)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    let result = fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap();
    db::group_db::delete_group(&fixture.author.db, &fixture.user, fixture.remote)
        .await
        .unwrap();
    let GroupLifecycle::Deleted { deletions } = db::causal_group_lifecycle::get_state(
        &fixture.author.db,
        &fixture.user,
        &fixture.author_identity,
        fixture.remote,
    )
    .await
    .unwrap()
    .lifecycle
    else {
        panic!("expected deletion")
    };
    fixture
        .sender
        .publish_operation(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            &deletions[0],
        )
        .await
        .unwrap();
    // The stored deletion time is part of the lifecycle guard, so it cannot be backdated;
    // retention compares whole seconds, so wait until the deletion is strictly older than now.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(
        db::group_db::purge_trashed_groups(&fixture.hub.db, 0)
            .await
            .unwrap(),
        1
    );
    let hub_user = db::user_db::get_user_by_email(&fixture.hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let replacement =
        db::group_db::add_empty_group(&fixture.hub.db, &hub_user, "replacement", None)
            .await
            .unwrap();
    assert!(
        db::group_db::edit_group(
            &fixture.hub.db,
            &hub_user,
            replacement.id,
            "replacement",
            None,
            Some(&fixture.global)
        )
        .await
        .is_err(),
        "purged identity cannot be assigned to a different birth"
    );
    fixture
        .client
        .publish_queued(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &result.outbox_id.unwrap(),
        )
        .await
        .unwrap();
    let replacement_global: String =
        sqlx::query_scalar("SELECT group_unique_global_id FROM models_group WHERE group_id=?")
            .bind(replacement.id)
            .fetch_one(&*fixture.hub.db)
            .await
            .unwrap();
    assert_eq!(replacement_global, replacement.unique_global_id);
    let (group, deleted): (i64, Option<String>) = sqlx::query_as("SELECT group_id,group_deleted_at FROM models_group WHERE group_user_id=? AND group_unique_global_id=?").bind(hub_user.id).bind(&fixture.global).fetch_one(&*fixture.hub.db).await.unwrap();
    assert_eq!(deleted, None);
    let member: Option<i64> = sqlx::query_scalar(
        "SELECT model_group_id FROM models WHERE model_user_id=? AND model_unique_global_id=?",
    )
    .bind(hub_user.id)
    .bind(local_global)
    .fetch_one(&*fixture.hub.db)
    .await
    .unwrap();
    assert_eq!(member, Some(group));
    assert!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM group_deletion_records WHERE user_id=? AND group_global_id=?"
        )
        .bind(hub_user.id)
        .bind(&fixture.global)
        .fetch_one(&*fixture.hub.db)
        .await
        .unwrap()
            > 0
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Check both raw-publication and missing-baseline paths on authenticated peers.
async fn populated_identity_link_cannot_bypass_its_baseline() {
    let fixture = fixture_with_remote_model(true).await;
    add_imported_local_model(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    let linked = fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap();
    let item: String =
        sqlx::query_scalar("SELECT item FROM sync_publication_outbox WHERE outbox_id=?")
            .bind(linked.outbox_id.as_deref().unwrap())
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    let wire: serde_json::Value = serde_json::from_str(&item).unwrap();
    let mut missing = wire.clone();
    missing["identity_link"]
        .as_object_mut()
        .unwrap()
        .remove("baseline");
    assert!(db::sync_transport::observations::parse(&missing).is_err());
    for counter in ["0", "01", "-1", "9223372036854775808"] {
        let mut malformed = wire.clone();
        malformed["identity_link"]["baseline"]["counter"] = counter.into();
        assert!(db::sync_transport::observations::parse(&malformed).is_err());
    }
    let mut malformed = wire;
    malformed["identity_link"]["baseline"]["writer_id"] = "not-a-writer".into();
    assert!(db::sync_transport::observations::parse(&malformed).is_err());
    let hub_user = db::user_db::get_user_by_email(&fixture.hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
        .bind(hub_user.id)
        .fetch_one(&*fixture.hub.db)
        .await
        .unwrap();
    assert!(
        fixture
            .client
            .publish_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &linked.event
            )
            .await
            .is_err(),
        "a populated identity link cannot be published as a standalone empty observation"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*fixture.hub.db)
            .await
            .unwrap(),
        before
    );
    fixture
        .client
        .publish_queued(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &linked.outbox_id.unwrap(),
        )
        .await
        .unwrap();
    let published: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*fixture.hub.db)
            .await
            .unwrap();
    assert!(
        fixture
            .client
            .publish_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &linked.event
            )
            .await
            .is_err(),
        "even after its full batch is present, a populated observation cannot be republished as an empty root"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*fixture.hub.db)
            .await
            .unwrap(),
        published
    );
    let (_third_dir, third) = crate::test_support::app_state().await;
    let third_identity = db::user_db::get_sync_identity(&third.db, fixture.user.id)
        .await
        .unwrap()
        .unwrap();
    let third_client = AuthenticatedHub::pair(
        &third,
        &fixture.user,
        &third_identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let auth = db::sync_transport::pull::Authorization::capture(
        &third.db,
        &fixture.user,
        &third_identity,
        &fixture.server.origin(),
        &fixture.token,
    )
    .await
    .unwrap();
    let (stream, _, _) = auth.position(&third.db).await.unwrap();
    sqlx::query("UPDATE sync_pull_streams SET last_applied=1 WHERE stream_id=?")
        .bind(&stream)
        .execute(&*third.db)
        .await
        .unwrap();
    assert!(
        third_client
            .pull_next(&third, &fixture.user, &third_identity)
            .await
            .is_err(),
        "the linked batch must not install if its baseline is missing"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT last_applied FROM sync_pull_streams WHERE stream_id=?"
        )
        .bind(stream)
        .fetch_one(&*third.db)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*third.db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models_group WHERE group_user_id=?")
            .bind(fixture.user.id)
            .fetch_one(&*third.db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models WHERE model_user_id=?")
            .bind(fixture.user.id)
            .fetch_one(&*third.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Both explicit choices must durably queue and relay their original observation.
async fn empty_identity_choices_atomically_queue_original_observations() {
    for same_group in [true, false] {
        let fixture = fixture().await;
        let choice = fixture
            .client
            .prepare_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                fixture.local,
            )
            .await
            .unwrap();
        let (outbox, event) = if same_group {
            let result = fixture
                .client
                .confirm_group_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                )
                .await
                .unwrap();
            (
                result
                    .outbox_id
                    .expect("confirmed identity must be queued atomically"),
                result.event,
            )
        } else {
            let result = fixture
                .client
                .separate_group_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                )
                .await
                .unwrap();
            (
                result
                    .outbox_id
                    .expect("separated identity must be queued atomically"),
                result.event,
            )
        };
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_publication_outbox WHERE outbox_id=? AND ack_sequence IS NULL").bind(&outbox).fetch_one(&*fixture.receiver.db).await.unwrap(), 1);
        if same_group {
            let retry = fixture
                .client
                .confirm_group_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                )
                .await
                .unwrap();
            assert!(retry.already_present);
            assert_eq!(retry.outbox_id.as_deref(), Some(outbox.as_str()));
            assert_eq!(retry.event, event);
        } else {
            let retry = fixture
                .client
                .separate_group_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                )
                .await
                .unwrap();
            assert!(retry.already_present);
            assert_eq!(retry.outbox_id.as_deref(), Some(outbox.as_str()));
            assert_eq!(retry.event, event);
        }
        fixture
            .client
            .publish_queued(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &outbox,
            )
            .await
            .unwrap();
        let (_third_dir, third) = crate::test_support::app_state().await;
        let third_identity = db::user_db::get_sync_identity(&third.db, fixture.user.id)
            .await
            .unwrap()
            .unwrap();
        let third_client = AuthenticatedHub::pair(
            &third,
            &fixture.user,
            &third_identity,
            &fixture.server.origin(),
            &fixture.token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
        )
        .await
        .unwrap();
        third_client
            .pull_next(&third, &fixture.user, &third_identity)
            .await
            .unwrap();
        third_client
            .pull_next(&third, &fixture.user, &third_identity)
            .await
            .unwrap();
        let groups: Vec<(i64, String)> = sqlx::query_as("SELECT group_id,group_name FROM models_group WHERE group_user_id=? ORDER BY group_name").bind(fixture.user.id).fetch_all(&*third.db).await.unwrap();
        assert_eq!(groups.len(), if same_group { 1 } else { 2 });
        if same_group {
            let state = db::causal_group_lifecycle::get_state(
                &third.db,
                &fixture.user,
                &third_identity,
                groups[0].0,
            )
            .await
            .unwrap();
            let db::causal_group_rename::NameState::Conflict { choices, .. } = state.name else {
                panic!("independent names must remain a choice")
            };
            let mut names: Vec<_> = choices.iter().map(|value| value.name.as_str()).collect();
            names.sort_unstable();
            assert_eq!(names, ["local name", "remote name"]);
        } else {
            assert_eq!(
                groups
                    .iter()
                    .map(|(_, name)| name.as_str())
                    .collect::<Vec<_>>(),
                ["local name", "remote name"]
            );
        }
    }
}

#[tokio::test]
async fn identity_name_choice_queues_original_operation_and_retries_after_later_edit() {
    Box::pin(assert_identity_name_choice_queues_and_retries(false)).await;
}

#[tokio::test]
async fn populated_identity_name_choice_relays_without_losing_models() {
    Box::pin(assert_identity_name_choice_queues_and_retries(true)).await;
}

#[allow(clippy::too_many_lines)] // A selected name must queue atomically, retry without reapplying and relay to another device.
async fn assert_identity_name_choice_queues_and_retries(populated: bool) {
    let fixture = fixture_with_remote_model(populated).await;
    if populated {
        add_imported_local_model(&fixture).await;
    }
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    let linked = fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap();
    let NameState::Conflict { token, .. } = db::causal_group_lifecycle::get_state(
        &fixture.receiver.db,
        &fixture.user,
        &fixture.receiver_identity,
        fixture.local,
    )
    .await
    .unwrap()
    .name
    else {
        panic!("expected choice")
    };
    let NamePublication::Queued {
        outbox_id,
        event,
        already_present,
        state,
    } = fixture
        .client
        .resolve_group_identity_name(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
            &token,
            "local name",
        )
        .await
        .unwrap()
    else {
        panic!("current choice must queue")
    };
    assert!(!already_present);
    assert!(matches!(state, NameState::Ready { ref name, .. } if name == "local name"));
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_publication_outbox WHERE outbox_id=? AND ack_sequence IS NULL").bind(&outbox_id).fetch_one(&*fixture.receiver.db).await.unwrap(), 1);
    db::group_db::edit_group(
        &fixture.receiver.db,
        &fixture.user,
        fixture.local,
        "later local",
        None,
        None,
    )
    .await
    .unwrap();
    let NamePublication::Queued {
        outbox_id: retried,
        event: retried_event,
        already_present,
        state,
    } = fixture
        .client
        .resolve_group_identity_name(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
            &token,
            "local name",
        )
        .await
        .unwrap()
    else {
        panic!("exact retry must return original receipt")
    };
    assert!(already_present);
    assert_eq!(retried, outbox_id);
    assert_eq!(retried_event, event);
    assert!(
        matches!(state, NameState::Ready { ref name, .. } if name == "later local"),
        "retry reports current state, not stale original projection"
    );
    fixture
        .client
        .publish_queued(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &linked.outbox_id.unwrap(),
        )
        .await
        .unwrap();
    fixture
        .client
        .publish_queued(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &outbox_id,
        )
        .await
        .unwrap();
    let event_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events WHERE user_id=?")
        .bind(fixture.user.id)
        .fetch_one(&*fixture.receiver.db)
        .await
        .unwrap();
    assert!(
        matches!(fixture.client.resolve_group_identity_name(&fixture.receiver, &fixture.user, &fixture.receiver_identity, &choice.decision_id, &token, "local name").await.unwrap(), NamePublication::Queued { ref outbox_id, already_present: true, .. } if outbox_id == &retried)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM causal_events WHERE user_id=?")
            .bind(fixture.user.id)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        event_count,
        "an acknowledged retry cannot author a second operation"
    );
    for _ in 0..3 {
        fixture
            .sender
            .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
            .await
            .unwrap();
    }
    assert!(
        matches!(db::causal_group_lifecycle::get_state(&fixture.author.db, &fixture.user, &fixture.author_identity, fixture.remote).await.unwrap().name, NameState::Ready { ref name, .. } if name == "local name")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM models WHERE model_user_id=? AND model_group_id=?"
        )
        .bind(fixture.user.id)
        .bind(fixture.remote)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        if populated { 2 } else { 0 }
    );
}

#[tokio::test]
async fn empty_identity_queue_failure_rolls_back_choice_and_keeps_local_editing() {
    for same_group in [true, false] {
        let fixture = fixture().await;
        let choice = fixture
            .client
            .prepare_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                fixture.local,
            )
            .await
            .unwrap();
        sqlx::query("CREATE TRIGGER fail_empty_choice_outbox BEFORE INSERT ON sync_publication_outbox BEGIN SELECT RAISE(ABORT,'injected failure'); END").execute(&*fixture.receiver.db).await.unwrap();
        let failed = if same_group {
            fixture
                .client
                .confirm_group_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                )
                .await
                .is_err()
        } else {
            fixture
                .client
                .separate_group_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                )
                .await
                .is_err()
        };
        assert!(failed);
        let group: (String, Option<String>) = sqlx::query_as(
            "SELECT group_unique_global_id,group_birth_id FROM models_group WHERE group_id=?",
        )
        .bind(fixture.local)
        .fetch_one(&*fixture.receiver.db)
        .await
        .unwrap();
        assert_eq!(group, (fixture.global.clone(), None));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models_group WHERE group_user_id=?")
                .bind(fixture.user.id)
                .fetch_one(&*fixture.receiver.db)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
                .fetch_one(&*fixture.receiver.db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_publication_outbox")
                .fetch_one(&*fixture.receiver.db)
                .await
                .unwrap(),
            0
        );
        db::group_db::edit_group(
            &fixture.receiver.db,
            &fixture.user,
            fixture.local,
            "local edit works",
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT group_name FROM models_group WHERE group_id=?")
                .bind(fixture.local)
                .fetch_one(&*fixture.receiver.db)
                .await
                .unwrap(),
            "local edit works"
        );
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Test a transaction failure and retry around an existing conflict.
async fn identity_name_queue_failure_keeps_conflict_and_allows_retry() {
    let fixture = fixture().await;
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap();
    let NameState::Conflict { token, .. } = db::causal_group_lifecycle::get_state(
        &fixture.receiver.db,
        &fixture.user,
        &fixture.receiver_identity,
        fixture.local,
    )
    .await
    .unwrap()
    .name
    else {
        panic!("expected conflict")
    };
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events WHERE user_id=?")
        .bind(fixture.user.id)
        .fetch_one(&*fixture.receiver.db)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_name_outbox BEFORE INSERT ON sync_publication_outbox BEGIN SELECT RAISE(ABORT,'injected failure'); END").execute(&*fixture.receiver.db).await.unwrap();
    assert!(
        fixture
            .client
            .resolve_group_identity_name(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                &token,
                "local name"
            )
            .await
            .is_err()
    );
    assert!(matches!(
        db::causal_group_lifecycle::get_state(
            &fixture.receiver.db,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local
        )
        .await
        .unwrap()
        .name,
        NameState::Conflict { .. }
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM causal_events WHERE user_id=?")
            .bind(fixture.user.id)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        events
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE user_id=?"
        )
        .bind(fixture.user.id)
        .fetch_one(&*fixture.receiver.db)
        .await
        .unwrap(),
        1
    );
    sqlx::query("DROP TRIGGER fail_name_outbox")
        .execute(&*fixture.receiver.db)
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .client
            .resolve_group_identity_name(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                &token,
                "local name"
            )
            .await
            .unwrap(),
        NamePublication::Queued {
            already_present: false,
            ..
        }
    ));
}

#[tokio::test]
async fn full_identity_outbox_does_not_block_ordinary_local_edits() {
    let fixture = fixture().await;
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    sqlx::query("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<4096) INSERT INTO sync_publication_outbox(outbox_id,user_id,local_instance_id,local_account_id,server_instance_id,server_account_id,connection_id,authority_writer_id,writer_id,writer_nonce,event_counter,kind,item_id,digest,item) SELECT printf('%032x',i),?,'instance','account','server','server-account','connection','authority','writer','nonce','1','group_operation',printf('%032x',i),'digest','{}' FROM n").bind(fixture.user.id).execute(&*fixture.receiver.db).await.unwrap();
    assert!(
        fixture
            .client
            .confirm_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id
            )
            .await
            .is_err(),
        "a full queue must not partially apply the identity choice"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    let birth: Option<String> =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id=?")
            .bind(fixture.local)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    assert_eq!(birth, None);
    db::group_db::edit_group(
        &fixture.receiver.db,
        &fixture.user,
        fixture.local,
        "still editable",
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT group_name FROM models_group WHERE group_id=?")
            .bind(fixture.local)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        "still editable"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Verify stale, unoffered and foreign choices without changing the conflict.
async fn identity_name_resolution_rejects_stale_unoffered_and_foreign_choices() {
    let fixture = fixture().await;
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap();
    let NameState::Conflict { token, .. } = db::causal_group_lifecycle::get_state(
        &fixture.receiver.db,
        &fixture.user,
        &fixture.receiver_identity,
        fixture.local,
    )
    .await
    .unwrap()
    .name
    else {
        panic!("expected conflict")
    };
    let mut stale = token.clone();
    stale.revision += 1;
    assert!(matches!(
        fixture
            .client
            .resolve_group_identity_name(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                &stale,
                "local name"
            )
            .await
            .unwrap(),
        NamePublication::Stale { .. }
    ));
    assert!(
        fixture
            .client
            .resolve_group_identity_name(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                &token,
                "not offered"
            )
            .await
            .is_err()
    );
    let other_id = db::user_db::add_user(
        &fixture.receiver.db,
        "other",
        "name-choice-other@example.com",
        &db::random_hex_32(),
    )
    .await
    .unwrap();
    let other = db::user_db::get_user_by_id(&fixture.receiver.db, other_id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = db::user_db::get_sync_identity(&fixture.receiver.db, other_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        fixture
            .client
            .resolve_group_identity_name(
                &fixture.receiver,
                &other,
                &other_identity,
                &choice.decision_id,
                &token,
                "local name"
            )
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE user_id=?"
        )
        .bind(fixture.user.id)
        .fetch_one(&*fixture.receiver.db)
        .await
        .unwrap(),
        1
    );
    assert!(matches!(
        db::causal_group_lifecycle::get_state(
            &fixture.receiver.db,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local
        )
        .await
        .unwrap()
        .name,
        NameState::Conflict { .. }
    ));
}

#[tokio::test]
async fn discarded_empty_identity_choice_cannot_retry_as_queued() {
    for same_group in [true, false] {
        let fixture = fixture().await;
        let choice = fixture
            .client
            .prepare_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                fixture.local,
            )
            .await
            .unwrap();
        let outbox = if same_group {
            fixture
                .client
                .confirm_group_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                )
                .await
                .unwrap()
                .outbox_id
                .unwrap()
        } else {
            fixture
                .client
                .separate_group_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                )
                .await
                .unwrap()
                .outbox_id
                .unwrap()
        };
        discard_queued_publication(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &outbox,
        )
        .await
        .unwrap();
        let failed = if same_group {
            fixture
                .client
                .confirm_group_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                )
                .await
                .is_err()
        } else {
            fixture
                .client
                .separate_group_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                )
                .await
                .is_err()
        };
        assert!(
            failed,
            "a discarded pending event must not be reported as queued"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM sync_publication_outbox WHERE outbox_id=?"
            )
            .bind(&outbox)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn discarded_name_resolution_cannot_retry_as_queued() {
    let fixture = fixture().await;
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap();
    let NameState::Conflict { token, .. } = db::causal_group_lifecycle::get_state(
        &fixture.receiver.db,
        &fixture.user,
        &fixture.receiver_identity,
        fixture.local,
    )
    .await
    .unwrap()
    .name
    else {
        panic!("expected conflict")
    };
    let NamePublication::Queued { outbox_id, .. } = fixture
        .client
        .resolve_group_identity_name(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
            &token,
            "local name",
        )
        .await
        .unwrap()
    else {
        panic!("expected queued choice")
    };
    discard_queued_publication(
        &fixture.receiver,
        &fixture.user,
        &fixture.receiver_identity,
        &outbox_id,
    )
    .await
    .unwrap();
    assert!(
        fixture
            .client
            .resolve_group_identity_name(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                &token,
                "local name"
            )
            .await
            .is_err(),
        "a discarded pending name resolution must not be reported as queued"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE outbox_id=?"
        )
        .bind(&outbox_id)
        .fetch_one(&*fixture.receiver.db)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Retry is bound to the original authorization even if the URL and token are reused.
async fn name_resolution_retry_after_relink_cannot_reuse_old_decision_authority() {
    let fixture = fixture().await;
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap();
    let NameState::Conflict { token, .. } = db::causal_group_lifecycle::get_state(
        &fixture.receiver.db,
        &fixture.user,
        &fixture.receiver_identity,
        fixture.local,
    )
    .await
    .unwrap()
    .name
    else {
        panic!("expected conflict")
    };
    let NamePublication::Queued {
        outbox_id, event, ..
    } = fixture
        .client
        .resolve_group_identity_name(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
            &token,
            "local name",
        )
        .await
        .unwrap()
    else {
        panic!("expected queued resolution")
    };
    db::user_db::clear_user_sync(&fixture.receiver.db, fixture.user.id)
        .await
        .unwrap();
    let relinked = AuthenticatedHub::pair(
        &fixture.receiver,
        &fixture.user,
        &fixture.receiver_identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert!(
        relinked
            .resolve_group_identity_name(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                &token,
                "local name"
            )
            .await
            .is_err(),
        "old choice cannot be rebound to a new link"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE outbox_id=?"
        )
        .bind(outbox_id)
        .fetch_one(&*fixture.receiver.db)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM causal_events WHERE user_id=? AND writer_id=? AND counter=?"
        )
        .bind(fixture.user.id)
        .bind(event.writer_id)
        .bind(event.counter.parse::<i64>().unwrap())
        .fetch_one(&*fixture.receiver.db)
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn populated_observation_cannot_use_empty_no_stage_outbox_path() {
    let fixture = fixture().await;
    let (model, path) = add_imported_local_model(&fixture).await;
    let outbox = fixture
        .client
        .queue_observation(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    let item: String =
        sqlx::query_scalar("SELECT item FROM sync_publication_outbox WHERE outbox_id=?")
            .bind(&outbox)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    let item: serde_json::Value = serde_json::from_str(&item).unwrap();
    let root = parse_causal_envelope(item["group"]["envelope"].as_str().unwrap()).unwrap();
    discard_queued_publication(
        &fixture.receiver,
        &fixture.user,
        &fixture.receiver_identity,
        &outbox,
    )
    .await
    .unwrap();
    let auth = db::sync_transport::pull::Authorization::capture(
        &fixture.receiver.db,
        &fixture.user,
        &fixture.receiver_identity,
        &fixture.server.origin(),
        &fixture.token,
    )
    .await
    .unwrap();
    assert!(
        db::sync_transport::outbox::queue(&fixture.receiver.db, &auth, &root.event, &item)
            .await
            .is_err(),
        "a populated observation cannot bypass certified staged content"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE user_id=?"
        )
        .bind(fixture.user.id)
        .fetch_one(&*fixture.receiver.db)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(model)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        fixture.local
    );
    assert_eq!(std::fs::read(path).unwrap(), b"xyz");
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Two receiving devices must queue independent choices and retain both when they conflict.
async fn received_name_offers_queue_concurrent_choices_on_original_and_third_devices() {
    for populated in [false, true] {
        let fixture = fixture_with_remote_model(populated).await;
        if populated {
            add_imported_local_model(&fixture).await;
        }
        let choice = fixture
            .client
            .prepare_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                fixture.local,
            )
            .await
            .unwrap();
        let linked = fixture
            .client
            .confirm_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
            )
            .await
            .unwrap();
        assert!(
            fixture
                .client
                .prepare_received_group_name(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    fixture.local
                )
                .await
                .is_err(),
            "a local observation is not an authenticated received identity receipt"
        );
        fixture
            .client
            .publish_queued(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &linked.outbox_id.unwrap(),
            )
            .await
            .unwrap();
        fixture
            .sender
            .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
            .await
            .unwrap();
        fixture
            .sender
            .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
            .await
            .unwrap();
        let (_third_dir, third) = crate::test_support::app_state().await;
        let third_identity = db::user_db::get_sync_identity(&third.db, fixture.user.id)
            .await
            .unwrap()
            .unwrap();
        let third_client = AuthenticatedHub::pair(
            &third,
            &fixture.user,
            &third_identity,
            &fixture.server.origin(),
            &fixture.token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
        )
        .await
        .unwrap();
        third_client
            .pull_next(&third, &fixture.user, &third_identity)
            .await
            .unwrap();
        third_client
            .pull_next(&third, &fixture.user, &third_identity)
            .await
            .unwrap();
        let third_group: i64 = sqlx::query_scalar(
            "SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
        )
        .bind(fixture.user.id)
        .bind(&fixture.global)
        .fetch_one(&*third.db)
        .await
        .unwrap();
        let offer_a = fixture
            .sender
            .prepare_received_group_name(
                &fixture.author,
                &fixture.user,
                &fixture.author_identity,
                fixture.remote,
            )
            .await
            .unwrap();
        let offer_c = third_client
            .prepare_received_group_name(&third, &fixture.user, &third_identity, third_group)
            .await
            .unwrap();
        let mut offered: Vec<_> = offer_a
            .choices
            .iter()
            .map(|choice| choice.name.as_str())
            .collect();
        offered.sort_unstable();
        assert_eq!(offered, ["local name", "remote name"]);
        let NamePublication::Queued {
            outbox_id: queue_a, ..
        } = fixture
            .sender
            .resolve_received_group_name(
                &fixture.author,
                &fixture.user,
                &fixture.author_identity,
                &offer_a.choice_id,
                "local name",
            )
            .await
            .unwrap()
        else {
            panic!("author choice should queue")
        };
        let NamePublication::Queued {
            outbox_id: queue_c, ..
        } = third_client
            .resolve_received_group_name(
                &third,
                &fixture.user,
                &third_identity,
                &offer_c.choice_id,
                "remote name",
            )
            .await
            .unwrap()
        else {
            panic!("third-device choice should queue")
        };
        fixture
            .sender
            .publish_queued(
                &fixture.author,
                &fixture.user,
                &fixture.author_identity,
                &queue_a,
            )
            .await
            .unwrap();
        third_client
            .publish_queued(&third, &fixture.user, &third_identity, &queue_c)
            .await
            .unwrap();
        for _ in 0..2 {
            fixture
                .sender
                .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
                .await
                .unwrap();
            third_client
                .pull_next(&third, &fixture.user, &third_identity)
                .await
                .unwrap();
        }
        for (state, identity, group) in [
            (&fixture.author, &fixture.author_identity, fixture.remote),
            (&third, &third_identity, third_group),
        ] {
            let NameState::Conflict { choices, .. } =
                db::causal_group_lifecycle::get_state(&state.db, &fixture.user, identity, group)
                    .await
                    .unwrap()
                    .name
            else {
                panic!("concurrent choices must remain a conflict")
            };
            let mut names: Vec<_> = choices.iter().map(|choice| choice.name.as_str()).collect();
            names.sort_unstable();
            assert_eq!(names, ["local name", "remote name"]);
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM models WHERE model_user_id=? AND model_group_id=?"
                )
                .bind(fixture.user.id)
                .bind(group)
                .fetch_one(&*state.db)
                .await
                .unwrap(),
                if populated { 2 } else { 0 }
            );
        }
    }
}

#[tokio::test]
async fn received_name_offer_rechecks_frontier_before_resolving() {
    let (fixture, decision) = Box::pin(received_name_fixture(false)).await;
    let offer = fixture
        .sender
        .prepare_received_group_name(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            fixture.remote,
        )
        .await
        .unwrap();
    let NameState::Conflict { token, .. } = db::causal_group_lifecycle::get_state(
        &fixture.receiver.db,
        &fixture.user,
        &fixture.receiver_identity,
        fixture.local,
    )
    .await
    .unwrap()
    .name
    else {
        panic!("origin must retain its name conflict")
    };
    let NamePublication::Queued { outbox_id, .. } = fixture
        .client
        .resolve_group_identity_name(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &decision,
            &token,
            "local name",
        )
        .await
        .unwrap()
    else {
        panic!("origin choice must queue")
    };
    fixture
        .client
        .publish_queued(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &outbox_id,
        )
        .await
        .unwrap();
    fixture
        .sender
        .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap();
    assert!(
        matches!(
            fixture
                .sender
                .resolve_received_group_name(
                    &fixture.author,
                    &fixture.user,
                    &fixture.author_identity,
                    &offer.choice_id,
                    "remote name"
                )
                .await
                .unwrap(),
            NamePublication::Stale { .. }
        ),
        "received offer must not erase a newer branch"
    );
    assert!(
        matches!(db::causal_group_lifecycle::get_state(&fixture.author.db, &fixture.user, &fixture.author_identity, fixture.remote).await.unwrap().name, NameState::Ready { ref name, .. } if name == "local name")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE user_id=?"
        )
        .bind(fixture.user.id)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        1,
        "only the original baseline publication exists"
    );
}

#[tokio::test]
async fn received_name_queue_failure_rolls_back_conflict_and_retries() {
    let (fixture, _) = Box::pin(received_name_fixture(false)).await;
    let offer = fixture
        .sender
        .prepare_received_group_name(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            fixture.remote,
        )
        .await
        .unwrap();
    assert!(
        fixture
            .sender
            .resolve_received_group_name(
                &fixture.author,
                &fixture.user,
                &fixture.author_identity,
                &offer.choice_id,
                "not offered"
            )
            .await
            .is_err(),
        "only displayed name choices are valid"
    );
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events WHERE user_id=?")
        .bind(fixture.user.id)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_received_name BEFORE INSERT ON sync_publication_outbox BEGIN SELECT RAISE(ABORT,'injected'); END").execute(&*fixture.author.db).await.unwrap();
    assert!(
        fixture
            .sender
            .resolve_received_group_name(
                &fixture.author,
                &fixture.user,
                &fixture.author_identity,
                &offer.choice_id,
                "local name"
            )
            .await
            .is_err()
    );
    assert!(matches!(
        db::causal_group_lifecycle::get_state(
            &fixture.author.db,
            &fixture.user,
            &fixture.author_identity,
            fixture.remote
        )
        .await
        .unwrap()
        .name,
        NameState::Conflict { .. }
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM causal_events WHERE user_id=?")
            .bind(fixture.user.id)
            .fetch_one(&*fixture.author.db)
            .await
            .unwrap(),
        before
    );
    sqlx::query("DROP TRIGGER fail_received_name")
        .execute(&*fixture.author.db)
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .sender
            .resolve_received_group_name(
                &fixture.author,
                &fixture.user,
                &fixture.author_identity,
                &offer.choice_id,
                "local name"
            )
            .await
            .unwrap(),
        NamePublication::Queued {
            already_present: false,
            ..
        }
    ));
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Reopen, exact retry, conflicting retry and discard must preserve the same receipt.
async fn received_name_choice_retries_after_database_reopen_then_reports_discard() {
    let (fixture, _) = Box::pin(received_name_fixture(false)).await;
    let offer = fixture
        .sender
        .prepare_received_group_name(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            fixture.remote,
        )
        .await
        .unwrap();
    let NamePublication::Queued {
        outbox_id, event, ..
    } = fixture
        .sender
        .resolve_received_group_name(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            &offer.choice_id,
            "local name",
        )
        .await
        .unwrap()
    else {
        panic!("expected queued choice")
    };
    let author_dir = fixture.dirs[1].path();
    let db =
        db::db_context::setup_db(&author_dir.join("db.sqlite"), &author_dir.join("backups")).await;
    let reopened = service::AppState {
        db: Arc::new(db),
        configuration: std::sync::Mutex::new(fixture.author.get_configuration()),
        import_mutex: Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: fixture.author.app_data_path.clone(),
    };
    let client = AuthenticatedHub::pair(
        &reopened,
        &fixture.user,
        &fixture.author_identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let NamePublication::Queued {
        outbox_id: retry_id,
        event: retry_event,
        already_present,
        ..
    } = client
        .resolve_received_group_name(
            &reopened,
            &fixture.user,
            &fixture.author_identity,
            &offer.choice_id,
            "local name",
        )
        .await
        .unwrap()
    else {
        panic!("exact retry must recover receipt")
    };
    assert!(already_present);
    assert_eq!((retry_id, retry_event), (outbox_id.clone(), event));
    assert!(
        client
            .resolve_received_group_name(
                &reopened,
                &fixture.user,
                &fixture.author_identity,
                &offer.choice_id,
                "remote name"
            )
            .await
            .is_err(),
        "a retry cannot choose a different name"
    );
    discard_queued_publication(
        &reopened,
        &fixture.user,
        &fixture.author_identity,
        &outbox_id,
    )
    .await
    .unwrap();
    assert!(
        client
            .resolve_received_group_name(
                &reopened,
                &fixture.user,
                &fixture.author_identity,
                &offer.choice_id,
                "local name"
            )
            .await
            .is_err(),
        "discarded publication cannot be reported as queued"
    );
}

#[tokio::test]
async fn received_name_offer_is_owner_and_link_bound() {
    let (fixture, _) = Box::pin(received_name_fixture(false)).await;
    let offer = fixture
        .sender
        .prepare_received_group_name(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            fixture.remote,
        )
        .await
        .unwrap();
    let other_id = db::user_db::add_user(
        &fixture.author.db,
        "other",
        "received-other@example.com",
        &db::random_hex_32(),
    )
    .await
    .unwrap();
    let other = db::user_db::get_user_by_id(&fixture.author.db, other_id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = db::user_db::get_sync_identity(&fixture.author.db, other_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        fixture
            .sender
            .resolve_received_group_name(
                &fixture.author,
                &other,
                &other_identity,
                &offer.choice_id,
                "local name"
            )
            .await
            .is_err()
    );
    db::user_db::clear_user_sync(&fixture.author.db, fixture.user.id)
        .await
        .unwrap();
    let relinked = AuthenticatedHub::pair(
        &fixture.author,
        &fixture.user,
        &fixture.author_identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert!(
        relinked
            .resolve_received_group_name(
                &fixture.author,
                &fixture.user,
                &fixture.author_identity,
                &offer.choice_id,
                "local name"
            )
            .await
            .is_err(),
        "an old offer cannot be rebound to the new link"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE user_id=? AND ack_sequence IS NULL"
        )
        .bind(fixture.user.id)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn received_name_offer_requires_actual_pull_receipt() {
    let (fixture, _) = Box::pin(received_name_fixture(false)).await;
    let deleted =
        sqlx::query("DELETE FROM sync_pull_receipts WHERE kind='group_identity_observation'")
            .execute(&*fixture.author.db)
            .await
            .unwrap()
            .rows_affected();
    assert_eq!(
        deleted, 1,
        "fixture must have received an identity observation"
    );
    assert!(
        fixture
            .sender
            .prepare_received_group_name(
                &fixture.author,
                &fixture.user,
                &fixture.author_identity,
                fixture.remote
            )
            .await
            .is_err(),
        "a stored root alone cannot substitute for an authenticated receipt"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE user_id=? AND ack_sequence IS NULL"
        )
        .bind(fixture.user.id)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn received_name_offer_rejects_altered_receipt_digest() {
    let (fixture, _) = Box::pin(received_name_fixture(false)).await;
    sqlx::query("CREATE TABLE test_receipt_backup AS SELECT * FROM sync_pull_receipts WHERE kind='group_identity_observation'").execute(&*fixture.author.db).await.unwrap();
    assert_eq!(
        sqlx::query("DELETE FROM sync_pull_receipts WHERE kind='group_identity_observation'")
            .execute(&*fixture.author.db)
            .await
            .unwrap()
            .rows_affected(),
        1
    );
    sqlx::query("INSERT INTO sync_pull_receipts(stream_id,sequence,kind,item_id,digest,issuer_connection_id,authority_writer_id,writer_id) SELECT stream_id,sequence,kind,item_id,?,issuer_connection_id,authority_writer_id,writer_id FROM test_receipt_backup").bind("0000000000000000000000000000000000000000000000000000000000000000").execute(&*fixture.author.db).await.unwrap();
    assert!(
        fixture
            .sender
            .prepare_received_group_name(
                &fixture.author,
                &fixture.user,
                &fixture.author_identity,
                fixture.remote
            )
            .await
            .is_err(),
        "a changed receipt digest cannot authorize a name offer"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE user_id=? AND ack_sequence IS NULL"
        )
        .bind(fixture.user.id)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn received_name_offer_rechecks_receipt_at_resolution() {
    let (fixture, _) = Box::pin(received_name_fixture(false)).await;
    let offer = fixture
        .sender
        .prepare_received_group_name(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            fixture.remote,
        )
        .await
        .unwrap();
    assert_eq!(
        sqlx::query("DELETE FROM sync_pull_receipts WHERE kind='group_identity_observation'")
            .execute(&*fixture.author.db)
            .await
            .unwrap()
            .rows_affected(),
        1
    );
    assert!(
        fixture
            .sender
            .resolve_received_group_name(
                &fixture.author,
                &fixture.user,
                &fixture.author_identity,
                &offer.choice_id,
                "local name"
            )
            .await
            .is_err(),
        "receipt removed after preparation must invalidate the offer"
    );
    assert!(matches!(
        db::causal_group_lifecycle::get_state(
            &fixture.author.db,
            &fixture.user,
            &fixture.author_identity,
            fixture.remote
        )
        .await
        .unwrap()
        .name,
        NameState::Conflict { .. }
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE user_id=? AND ack_sequence IS NULL"
        )
        .bind(fixture.user.id)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One delivery must yield an actionable identity choice and expose its queued publication separately from feed status.
async fn advance_one_offers_exact_identity_choice_and_separately_reports_pending_work() {
    for same_group in [true, false] {
        let fixture = fixture().await;
        let pending = fixture
            .client
            .pending_publications(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
            .await
            .unwrap();
        assert_eq!(pending.outbox_ids, [] as [String; 0]);
        let AdvanceOutcome::IdentityRequired(choice) = fixture
            .client
            .advance_one(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
            .await
            .unwrap()
        else {
            panic!("occupied unbound group must ask for identity choice")
        };
        assert_eq!(
            (&*choice.local_name, &*choice.remote_name),
            ("local name", "remote name")
        );
        let (group_name, birth): (String, Option<String>) =
            sqlx::query_as("SELECT group_name,group_birth_id FROM models_group WHERE group_id=?")
                .bind(fixture.local)
                .fetch_one(&*fixture.receiver.db)
                .await
                .unwrap();
        assert_eq!((group_name, birth), ("local name".into(), None));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
                .fetch_one(&*fixture.receiver.db)
                .await
                .unwrap(),
            0
        );
        let outbox = if same_group {
            fixture
                .client
                .confirm_group_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                )
                .await
                .unwrap()
                .outbox_id
                .unwrap()
        } else {
            fixture
                .client
                .separate_group_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                )
                .await
                .unwrap()
                .outbox_id
                .unwrap()
        };
        assert!(matches!(
            fixture
                .client
                .advance_one(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
                .await
                .unwrap(),
            AdvanceOutcome::FeedBoundary { .. }
        ));
        let pending = fixture
            .client
            .pending_publications(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
            .await
            .unwrap();
        assert_eq!(
            pending.outbox_ids,
            vec![outbox.clone()],
            "an end-of-feed result does not mean pending work is complete"
        );
        assert!(!pending.has_more);
        assert_eq!(pending.other_link_count, 0);
        fixture
            .client
            .publish_queued(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &outbox,
            )
            .await
            .unwrap();
        assert_eq!(
            fixture
                .client
                .pending_publications(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
                .await
                .unwrap()
                .outbox_ids,
            [] as [String; 0]
        );
        assert!(matches!(
            fixture
                .client
                .advance_one(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
                .await
                .unwrap(),
            AdvanceOutcome::AlreadyApplied { .. }
        ));
    }
}

#[tokio::test]
async fn advance_one_fetches_each_delivery_only_once() {
    for occupied in [true, false] {
        let count = Arc::new(AtomicUsize::new(0));
        let fixture = fixture_with_counter(FixtureOptions {
            counter: Some(count.clone()),
            ..FixtureOptions::default()
        })
        .await;
        if occupied {
            count.store(0, Ordering::SeqCst);
            let outcome = fixture
                .client
                .advance_one(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
                .await
                .unwrap();
            assert!(matches!(outcome, AdvanceOutcome::IdentityRequired(_)));
        } else {
            let (_third_dir, third) = crate::test_support::app_state().await;
            let third_identity = db::user_db::get_sync_identity(&third.db, fixture.user.id)
                .await
                .unwrap()
                .unwrap();
            let third_client = AuthenticatedHub::pair(
                &third,
                &fixture.user,
                &third_identity,
                &fixture.server.origin(),
                &fixture.token,
                HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
            )
            .await
            .unwrap();
            count.store(0, Ordering::SeqCst);
            let outcome = third_client
                .advance_one(&third, &fixture.user, &third_identity)
                .await
                .unwrap();
            assert!(matches!(outcome, AdvanceOutcome::Applied { .. }));
        }
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "classification and application must use the same authenticated feed response"
        );
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Paging must remain stable across reopen, deletion, different users and relinking.
async fn pending_publications_are_bounded_owner_scoped_and_survive_reopen() {
    let fixture = fixture().await;
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    let queued = fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap()
        .outbox_id
        .unwrap();
    sqlx::query("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<64) INSERT INTO sync_publication_outbox(outbox_id,user_id,local_instance_id,local_account_id,server_instance_id,server_account_id,connection_id,authority_writer_id,writer_id,writer_nonce,event_counter,kind,item_id,digest,item) SELECT printf('%032x',i),o.user_id,o.local_instance_id,o.local_account_id,o.server_instance_id,o.server_account_id,o.connection_id,o.authority_writer_id,o.writer_id,o.writer_nonce,o.event_counter,'group_operation',printf('%032x',i),o.digest,o.item FROM sync_publication_outbox o,n WHERE o.outbox_id=?").bind(&queued).execute(&*fixture.receiver.db).await.unwrap();
    sqlx::query("INSERT INTO sync_publication_outbox(outbox_id,user_id,local_instance_id,local_account_id,server_instance_id,server_account_id,connection_id,authority_writer_id,writer_id,writer_nonce,event_counter,kind,item_id,digest,item) SELECT ?,user_id,local_instance_id,local_account_id,server_instance_id,server_account_id,'different-link',authority_writer_id,writer_id,writer_nonce,event_counter,'group_operation',?,digest,item FROM sync_publication_outbox WHERE outbox_id=?").bind(db::random_hex_32()).bind(db::random_hex_32()).bind(&queued).execute(&*fixture.receiver.db).await.unwrap();
    let other_id = db::user_db::add_user(
        &fixture.receiver.db,
        "other",
        "pending-other@example.com",
        &db::random_hex_32(),
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO sync_publication_outbox(outbox_id,user_id,local_instance_id,local_account_id,server_instance_id,server_account_id,connection_id,authority_writer_id,writer_id,writer_nonce,event_counter,kind,item_id,digest,item) SELECT ?,?,local_instance_id,local_account_id,server_instance_id,server_account_id,connection_id,authority_writer_id,writer_id,writer_nonce,event_counter,'group_operation',?,digest,item FROM sync_publication_outbox WHERE outbox_id=?").bind(db::random_hex_32()).bind(other_id).bind(db::random_hex_32()).bind(&queued).execute(&*fixture.receiver.db).await.unwrap();
    let pending = fixture
        .client
        .pending_publications(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
        .await
        .unwrap();
    assert_eq!(pending.outbox_ids.len(), 64);
    assert!(pending.has_more);
    assert_eq!(
        pending.other_link_count, 1,
        "another owner's rows are never reported"
    );
    let receiver_dir = fixture.dirs[2].path();
    let db = db::db_context::setup_db(
        &receiver_dir.join("db.sqlite"),
        &receiver_dir.join("backups"),
    )
    .await;
    let reopened = service::AppState {
        db: Arc::new(db),
        configuration: std::sync::Mutex::new(fixture.receiver.get_configuration()),
        import_mutex: Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: fixture.receiver.app_data_path.clone(),
    };
    let client = AuthenticatedHub::pair(
        &reopened,
        &fixture.user,
        &fixture.receiver_identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let after = client
        .pending_publications(&reopened, &fixture.user, &fixture.receiver_identity)
        .await
        .unwrap();
    assert_eq!(after.outbox_ids, pending.outbox_ids);
    assert!(after.has_more);
    assert_eq!(after.other_link_count, 1);
    let cursor = after
        .next_cursor
        .clone()
        .expect("a truncated page needs a continuation cursor");
    let expected: Vec<String> = sqlx::query_scalar("SELECT outbox_id FROM sync_publication_outbox WHERE user_id=? AND connection_id=(SELECT connection_id FROM sync_transport_local_authorities WHERE user_id=?) AND ack_sequence IS NULL ORDER BY outbox_id").bind(fixture.user.id).bind(fixture.user.id).fetch_all(&*reopened.db).await.unwrap();
    assert_eq!(expected.len(), 65);
    let removed = after.outbox_ids.last().unwrap().clone();
    sqlx::query("DELETE FROM sync_publication_outbox WHERE outbox_id=?")
        .bind(&removed)
        .execute(&*reopened.db)
        .await
        .unwrap();
    let second = client
        .pending_publications_page(
            &reopened,
            &fixture.user,
            &fixture.receiver_identity,
            Some(&cursor),
        )
        .await
        .unwrap();
    assert!(!second.has_more);
    assert_eq!(second.next_cursor, None);
    assert_eq!(second.other_link_count, 1);
    let mut all = after.outbox_ids.clone();
    all.retain(|id| id != &removed);
    all.extend(second.outbox_ids);
    assert_eq!(
        all,
        expected
            .into_iter()
            .filter(|id| id != &removed)
            .collect::<Vec<_>>(),
        "deleting the cursor row cannot skip or duplicate other entries"
    );
    assert!(
        client
            .pending_publications_page(
                &reopened,
                &fixture.user,
                &fixture.receiver_identity,
                Some("malformed")
            )
            .await
            .is_err()
    );
    let other = db::user_db::get_user_by_id(&reopened.db, other_id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = db::user_db::get_sync_identity(&reopened.db, other_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        client
            .pending_publications_page(&reopened, &other, &other_identity, Some(&cursor))
            .await
            .is_err()
    );
    db::user_db::clear_user_sync(&reopened.db, fixture.user.id)
        .await
        .unwrap();
    let relinked = AuthenticatedHub::pair(
        &reopened,
        &fixture.user,
        &fixture.receiver_identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert!(
        relinked
            .pending_publications_page(
                &reopened,
                &fixture.user,
                &fixture.receiver_identity,
                Some(&cursor)
            )
            .await
            .is_err(),
        "a cursor from an old link cannot be reused"
    );
}

#[tokio::test]
async fn advance_one_never_skips_ambiguous_candidates_or_child_collisions() {
    for duplicate in [true, false] {
        let fixture = fixture_with_remote_model(!duplicate).await;
        if duplicate {
            sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('duplicate','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(fixture.user.id).bind(&fixture.global).execute(&*fixture.receiver.db).await.unwrap();
        } else {
            let (child_global, _) = fixture.child.clone().unwrap();
            let blob =
                db::blob_db::add_blob(&fixture.receiver.db, &db::random_hex_32(), "stl", 3, None)
                    .await
                    .unwrap();
            let model = db::model_db::add_model(
                &fixture.receiver.db,
                &fixture.user,
                "existing",
                blob,
                None,
                None,
            )
            .await
            .unwrap();
            sqlx::query("UPDATE models SET model_unique_global_id=? WHERE model_id=?")
                .bind(child_global)
                .bind(model)
                .execute(&*fixture.receiver.db)
                .await
                .unwrap();
        }
        assert!(
            fixture
                .client
                .advance_one(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
                .await
                .is_err(),
            "ambiguous identities must not be silently matched or skipped"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
                .fetch_one(&*fixture.receiver.db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MAX(last_applied),0) FROM sync_pull_streams"
            )
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT group_birth_id FROM models_group WHERE group_id=?"
            )
            .bind(fixture.local)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
            None
        );
    }
}

#[tokio::test]
async fn advance_one_rejects_link_changed_after_feed_response() {
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reached = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let fixture = fixture_with_counter(FixtureOptions {
        feed_gate: Some((armed.clone(), reached.clone(), release.clone())),
        ..FixtureOptions::default()
    })
    .await;
    armed.store(true, Ordering::SeqCst);
    let advance =
        fixture
            .client
            .advance_one(&fixture.receiver, &fixture.user, &fixture.receiver_identity);
    let unlink = async {
        reached.notified().await;
        db::user_db::clear_user_sync(&fixture.receiver.db, fixture.user.id)
            .await
            .unwrap();
        release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        Box::pin(async { tokio::join!(advance, unlink) }),
    )
    .await
    .expect("feed request must reach the barrier");
    assert!(result.is_err());
    let birth: Option<String> =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id=?")
            .bind(fixture.local)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    assert_eq!(birth, None);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(last_applied),0) FROM sync_pull_streams")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn advance_one_rejects_malformed_or_altered_feed_without_advancing() {
    let attack = Arc::new(AtomicUsize::new(0));
    let fixture = fixture_with_counter(FixtureOptions {
        attack: Some(attack.clone()),
        ..FixtureOptions::default()
    })
    .await;
    for mode in 1..=3 {
        attack.store(mode, Ordering::SeqCst);
        assert!(
            fixture
                .client
                .advance_one(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
                .await
                .is_err(),
            "malformed feed mode {mode} must fail"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MAX(last_applied),0) FROM sync_pull_streams"
            )
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
                .fetch_one(&*fixture.receiver.db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT group_birth_id FROM models_group WHERE group_id=?"
            )
            .bind(fixture.local)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
            None
        );
    }
}

#[tokio::test]
async fn advance_one_acknowledges_proven_source_echo_after_global_id_reuse() {
    let fixture = fixture_with_remote_model(true).await;
    let fresh = db::random_hex_32();
    db::group_db::edit_group(
        &fixture.author.db,
        &fixture.user,
        fixture.remote,
        "rekeyed original",
        None,
        Some(&fresh),
    )
    .await
    .unwrap();
    let replacement = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('replacement','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(fixture.user.id).bind(&fixture.global).execute(&*fixture.author.db).await.unwrap().last_insert_rowid();
    assert!(
        matches!(fixture.sender.advance_one(&fixture.author, &fixture.user, &fixture.author_identity).await.unwrap(), AdvanceOutcome::AlreadyApplied { ref sequence } if sequence == "1"),
        "a proven source echo cannot turn a later unrelated row into an identity choice"
    );
    let row: (String, Option<String>) =
        sqlx::query_as("SELECT group_name,group_birth_id FROM models_group WHERE group_id=?")
            .bind(replacement)
            .fetch_one(&*fixture.author.db)
            .await
            .unwrap();
    assert_eq!(row, ("replacement".into(), None));
    let original: (String, String) = sqlx::query_as(
        "SELECT group_name,group_unique_global_id FROM models_group WHERE group_id=?",
    )
    .bind(fixture.remote)
    .fetch_one(&*fixture.author.db)
    .await
    .unwrap();
    assert_eq!(original, ("rekeyed original".into(), fresh));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.author.db)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Selection must relay both original operations even when a later edit is already visible.
async fn selected_local_operations_preserve_original_events_and_relay_in_order() {
    let fixture = fixture_with_remote_model(true).await;
    db::group_db::edit_group(
        &fixture.author.db,
        &fixture.user,
        fixture.remote,
        "first",
        None,
        None,
    )
    .await
    .unwrap();
    db::group_db::edit_group(
        &fixture.author.db,
        &fixture.user,
        fixture.remote,
        "second",
        None,
        None,
    )
    .await
    .unwrap();
    let OperationSelection::Queued {
        operation_id: first_id,
        outbox_id: first,
        event: first_event,
    } = fixture
        .sender
        .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap()
    else {
        panic!("first authored operation must be selected")
    };
    let OperationSelection::Queued {
        operation_id: second_id,
        outbox_id: second,
        event: second_event,
    } = fixture
        .sender
        .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap()
    else {
        panic!("second authored operation must be selected")
    };
    assert_ne!(first_id, second_id);
    assert_ne!(first_event, second_event);
    fixture
        .sender
        .publish_queued(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            &first,
        )
        .await
        .unwrap();
    fixture
        .sender
        .publish_queued(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            &second,
        )
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .sender
            .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
            .await
            .unwrap(),
        OperationSelection::NoEligible { .. }
    ));
    let (_third_dir, third) = crate::test_support::app_state().await;
    let third_identity = db::user_db::get_sync_identity(&third.db, fixture.user.id)
        .await
        .unwrap()
        .unwrap();
    let third_client = AuthenticatedHub::pair(
        &third,
        &fixture.user,
        &third_identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    for sequence in ["1", "2", "3"] {
        assert!(
            matches!(third_client.advance_one(&third, &fixture.user, &third_identity).await.unwrap(), AdvanceOutcome::Applied { sequence: actual } if actual == sequence)
        );
    }
    let name: String = sqlx::query_scalar(
        "SELECT group_name FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(fixture.user.id)
    .bind(&fixture.global)
    .fetch_one(&*third.db)
    .await
    .unwrap();
    assert_eq!(name, "second");
    for event in [first_event, second_event] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM causal_events WHERE user_id=? AND writer_id=? AND counter=?"
            )
            .bind(fixture.user.id)
            .bind(event.writer_id)
            .bind(event.counter.parse::<i64>().unwrap())
            .fetch_one(&*third.db)
            .await
            .unwrap(),
            1,
            "relay retains the original authored dot"
        );
    }
}

#[tokio::test]
async fn selector_never_requeues_explicitly_discarded_operation() {
    let fixture = fixture().await;
    db::group_db::edit_group(
        &fixture.author.db,
        &fixture.user,
        fixture.remote,
        "offline rename",
        None,
        None,
    )
    .await
    .unwrap();
    let OperationSelection::Queued {
        outbox_id, event, ..
    } = fixture
        .sender
        .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap()
    else {
        panic!("operation should queue")
    };
    discard_queued_publication(
        &fixture.author,
        &fixture.user,
        &fixture.author_identity,
        &outbox_id,
    )
    .await
    .unwrap();
    assert!(
        matches!(
            fixture
                .sender
                .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
                .await
                .unwrap(),
            OperationSelection::NoEligible {
                discarded_operations: 1,
                ..
            }
        ),
        "explicit discard must remain a durable decision"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE outbox_id=?"
        )
        .bind(outbox_id)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM causal_events WHERE user_id=? AND writer_id=? AND counter=?"
        )
        .bind(fixture.user.id)
        .bind(event.writer_id)
        .bind(event.counter.parse::<i64>().unwrap())
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        1,
        "discard must retain original history"
    );
}

#[tokio::test]
async fn selector_queue_failure_keeps_local_edit_and_retries_original_event() {
    let fixture = fixture().await;
    db::group_db::edit_group(
        &fixture.author.db,
        &fixture.user,
        fixture.remote,
        "offline rename",
        None,
        None,
    )
    .await
    .unwrap();
    let NameState::Ready { token, .. } = db::causal_group_lifecycle::get_state(
        &fixture.author.db,
        &fixture.user,
        &fixture.author_identity,
        fixture.remote,
    )
    .await
    .unwrap()
    .name
    else {
        panic!("local rename should remain usable")
    };
    let original = token.frontier[0].clone();
    sqlx::query("CREATE TRIGGER fail_selector_outbox BEFORE INSERT ON sync_publication_outbox BEGIN SELECT RAISE(ABORT,'injected'); END").execute(&*fixture.author.db).await.unwrap();
    assert!(
        fixture
            .sender
            .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT group_name FROM models_group WHERE group_id=?")
            .bind(fixture.remote)
            .fetch_one(&*fixture.author.db)
            .await
            .unwrap(),
        "offline rename"
    );
    sqlx::query("DROP TRIGGER fail_selector_outbox")
        .execute(&*fixture.author.db)
        .await
        .unwrap();
    let OperationSelection::Queued { event, .. } = fixture
        .sender
        .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap()
    else {
        panic!("original event should remain eligible")
    };
    assert_eq!(event, original);
}

#[tokio::test]
async fn selector_full_queue_keeps_local_edit_selectable_after_space_frees() {
    let fixture = fixture().await;
    db::group_db::edit_group(
        &fixture.author.db,
        &fixture.user,
        fixture.remote,
        "first",
        None,
        None,
    )
    .await
    .unwrap();
    let OperationSelection::Queued {
        outbox_id: first, ..
    } = fixture
        .sender
        .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap()
    else {
        panic!("first rename should queue")
    };
    sqlx::query("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<4095) INSERT INTO sync_publication_outbox(outbox_id,user_id,local_instance_id,local_account_id,server_instance_id,server_account_id,connection_id,authority_writer_id,writer_id,writer_nonce,event_counter,kind,item_id,digest,item) SELECT printf('%032x',i),o.user_id,o.local_instance_id,o.local_account_id,o.server_instance_id,o.server_account_id,o.connection_id,o.authority_writer_id,o.writer_id,o.writer_nonce,o.event_counter,'group_operation',printf('%032x',i),o.digest,o.item FROM sync_publication_outbox o,n WHERE o.outbox_id=?").bind(&first).execute(&*fixture.author.db).await.unwrap();
    db::group_db::edit_group(
        &fixture.author.db,
        &fixture.user,
        fixture.remote,
        "second",
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        fixture
            .sender
            .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
            .await
            .is_err(),
        "queue capacity cannot be exceeded"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT group_name FROM models_group WHERE group_id=?")
            .bind(fixture.remote)
            .fetch_one(&*fixture.author.db)
            .await
            .unwrap(),
        "second",
        "ordinary edit remains committed"
    );
    sqlx::query("DELETE FROM sync_publication_outbox WHERE outbox_id=?")
        .bind("00000000000000000000000000000001")
        .execute(&*fixture.author.db)
        .await
        .unwrap();
    let OperationSelection::Queued { event, .. } = fixture
        .sender
        .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap()
    else {
        panic!("operation remains eligible after space frees")
    };
    let wire: String = sqlx::query_scalar(
        "SELECT envelope FROM causal_events WHERE user_id=? AND writer_id=? AND counter=?",
    )
    .bind(fixture.user.id)
    .bind(event.writer_id)
    .bind(event.counter.parse::<i64>().unwrap())
    .fetch_one(&*fixture.author.db)
    .await
    .unwrap();
    assert!(
        matches!(parse_causal_envelope(&wire).unwrap().action, Action::GroupRenamed { ref name } if name == "second")
    );
}

#[tokio::test]
async fn selector_never_reauthors_received_operations() {
    let fixture = fixture().await;
    db::group_db::edit_group(
        &fixture.author.db,
        &fixture.user,
        fixture.remote,
        "authored elsewhere",
        None,
        None,
    )
    .await
    .unwrap();
    let OperationSelection::Queued {
        outbox_id, event, ..
    } = fixture
        .sender
        .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap()
    else {
        panic!("source operation should queue")
    };
    fixture
        .sender
        .publish_queued(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            &outbox_id,
        )
        .await
        .unwrap();
    let (_third_dir, third) = crate::test_support::app_state().await;
    let identity = db::user_db::get_sync_identity(&third.db, fixture.user.id)
        .await
        .unwrap()
        .unwrap();
    let client = AuthenticatedHub::pair(
        &third,
        &fixture.user,
        &identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    client
        .advance_one(&third, &fixture.user, &identity)
        .await
        .unwrap();
    client
        .advance_one(&third, &fixture.user, &identity)
        .await
        .unwrap();
    assert!(
        matches!(
            client
                .select_next_operation(&third, &fixture.user, &identity)
                .await
                .unwrap(),
            OperationSelection::NoEligible { .. }
        ),
        "a relayed operation is not a new local authored change"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE user_id=?"
        )
        .bind(fixture.user.id)
        .fetch_one(&*third.db)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM causal_events WHERE user_id=? AND writer_id=? AND counter=?"
        )
        .bind(fixture.user.id)
        .bind(event.writer_id)
        .bind(event.counter.parse::<i64>().unwrap())
        .fetch_one(&*third.db)
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn selector_queues_complete_move_bundle_with_both_causal_events() {
    let fixture = fixture().await;
    let blob = db::blob_db::add_blob(&fixture.author.db, &db::random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let model = db::model_db::add_model(
        &fixture.author.db,
        &fixture.user,
        "movable",
        blob,
        None,
        None,
    )
    .await
    .unwrap();
    db::causal_group_lifecycle::enroll_pristine_model(
        &fixture.author.db,
        &fixture.user,
        &fixture.author_identity,
        model,
    )
    .await
    .unwrap();
    db::group_db::set_group_id_on_models(
        &fixture.author.db,
        &fixture.user,
        Some(fixture.remote),
        vec![model],
        None,
    )
    .await
    .unwrap();
    let OperationSelection::Queued { outbox_id, .. } = fixture
        .sender
        .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap()
    else {
        panic!("move should be selected")
    };
    let item: String =
        sqlx::query_scalar("SELECT item FROM sync_publication_outbox WHERE outbox_id=?")
            .bind(outbox_id)
            .fetch_one(&*fixture.author.db)
            .await
            .unwrap();
    let item: serde_json::Value = serde_json::from_str(&item).unwrap();
    let wires = item["envelopes"].as_array().unwrap();
    assert_eq!(
        wires.len(),
        2,
        "model move and group addition must be queued together"
    );
    let events: Vec<_> = wires
        .iter()
        .map(|wire| parse_causal_envelope(wire.as_str().unwrap()).unwrap())
        .collect();
    assert!(
        events
            .iter()
            .any(|event| matches!(event.action, Action::ModelMoved { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.action, Action::GroupMemberAdded { .. }))
    );
}

#[tokio::test]
async fn selector_reports_preexisting_untracked_history_without_reauthoring_it() {
    let fixture = fixture().await;
    db::group_db::edit_group(
        &fixture.author.db,
        &fixture.user,
        fixture.remote,
        "old operation",
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query(
            "DELETE FROM sync_local_operation_selections WHERE user_id=? AND outbox_id IS NULL"
        )
        .bind(fixture.user.id)
        .execute(&*fixture.author.db)
        .await
        .unwrap()
        .rows_affected(),
        1,
        "fixture emulates an operation from before selector migration"
    );
    assert!(
        matches!(
            fixture
                .sender
                .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
                .await
                .unwrap(),
            OperationSelection::NoEligible {
                untracked_operations: 1,
                ..
            }
        ),
        "preexisting journal history cannot acquire a fabricated delegation"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE user_id=? AND ack_sequence IS NULL"
        )
        .bind(fixture.user.id)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn selector_reports_old_link_operations_and_rejects_foreign_owner() {
    let fixture = fixture().await;
    db::group_db::edit_group(
        &fixture.author.db,
        &fixture.user,
        fixture.remote,
        "old link edit",
        None,
        None,
    )
    .await
    .unwrap();
    let other_id = db::user_db::add_user(
        &fixture.author.db,
        "other",
        "selector-other@example.com",
        &db::random_hex_32(),
    )
    .await
    .unwrap();
    let other = db::user_db::get_user_by_id(&fixture.author.db, other_id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = db::user_db::get_sync_identity(&fixture.author.db, other_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        fixture
            .sender
            .select_next_operation(&fixture.author, &other, &other_identity)
            .await
            .is_err()
    );
    db::user_db::clear_user_sync(&fixture.author.db, fixture.user.id)
        .await
        .unwrap();
    let relinked = AuthenticatedHub::pair(
        &fixture.author,
        &fixture.user,
        &fixture.author_identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert!(
        matches!(
            relinked
                .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
                .await
                .unwrap(),
            OperationSelection::NoEligible {
                other_link_operations: 1,
                ..
            }
        ),
        "old writer's operation cannot be silently rebound to new authorization"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE user_id=? AND ack_sequence IS NULL"
        )
        .bind(fixture.user.id)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn selector_blocks_when_untracked_model_metadata_changes_pause_scope() {
    let fixture = fixture_with_remote_model(true).await;
    db::group_db::edit_group(
        &fixture.author.db,
        &fixture.user,
        fixture.remote,
        "tracked rename",
        None,
        None,
    )
    .await
    .unwrap();
    let model: i64 = sqlx::query_scalar(
        "SELECT model_id FROM models WHERE model_user_id=? AND model_group_id=?",
    )
    .bind(fixture.user.id)
    .bind(fixture.remote)
    .fetch_one(&*fixture.author.db)
    .await
    .unwrap();
    sqlx::query("UPDATE models SET model_name='untracked metadata' WHERE model_id=?")
        .bind(model)
        .execute(&*fixture.author.db)
        .await
        .unwrap();
    assert!(
        fixture
            .sender
            .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
            .await
            .is_err(),
        "unsupported local drift cannot be silently reported as synced"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_publication_outbox WHERE user_id=? AND ack_sequence IS NULL"
        )
        .bind(fixture.user.id)
        .fetch_one(&*fixture.author.db)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT group_name FROM models_group WHERE group_id=?")
            .bind(fixture.remote)
            .fetch_one(&*fixture.author.db)
            .await
            .unwrap(),
        "tracked rename"
    );
}

#[tokio::test]
async fn selector_recovers_unsent_original_operation_after_reopen() {
    let fixture = fixture().await;
    db::group_db::edit_group(
        &fixture.author.db,
        &fixture.user,
        fixture.remote,
        "before restart",
        None,
        None,
    )
    .await
    .unwrap();
    let NameState::Ready { token, .. } = db::causal_group_lifecycle::get_state(
        &fixture.author.db,
        &fixture.user,
        &fixture.author_identity,
        fixture.remote,
    )
    .await
    .unwrap()
    .name
    else {
        panic!("expected original rename")
    };
    let expected = token.frontier[0].clone();
    let directory = fixture.dirs[1].path();
    let db =
        db::db_context::setup_db(&directory.join("db.sqlite"), &directory.join("backups")).await;
    let reopened = service::AppState {
        db: Arc::new(db),
        configuration: std::sync::Mutex::new(fixture.author.get_configuration()),
        import_mutex: Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: fixture.author.app_data_path.clone(),
    };
    let client = AuthenticatedHub::pair(
        &reopened,
        &fixture.user,
        &fixture.author_identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let OperationSelection::Queued {
        outbox_id, event, ..
    } = client
        .select_next_operation(&reopened, &fixture.user, &fixture.author_identity)
        .await
        .unwrap()
    else {
        panic!("unselected journal operation must survive reopen")
    };
    assert_eq!(event, expected);
    client
        .publish_queued(
            &reopened,
            &fixture.user,
            &fixture.author_identity,
            &outbox_id,
        )
        .await
        .unwrap();
    let (_third_dir, third) = crate::test_support::app_state().await;
    let third_identity = db::user_db::get_sync_identity(&third.db, fixture.user.id)
        .await
        .unwrap()
        .unwrap();
    let third_client = AuthenticatedHub::pair(
        &third,
        &fixture.user,
        &third_identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    third_client
        .advance_one(&third, &fixture.user, &third_identity)
        .await
        .unwrap();
    third_client
        .advance_one(&third, &fixture.user, &third_identity)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT group_name FROM models_group WHERE group_user_id=? AND group_unique_global_id=?"
        )
        .bind(fixture.user.id)
        .bind(&fixture.global)
        .fetch_one(&*third.db)
        .await
        .unwrap(),
        "before restart"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Deletion and restoration must each retain their original complete causal cut.
async fn selector_relays_delete_restore_with_original_author_context() {
    let fixture = fixture_with_remote_model(true).await;
    db::group_db::delete_group(&fixture.author.db, &fixture.user, fixture.remote)
        .await
        .unwrap();
    let OperationSelection::Queued {
        outbox_id: deleted,
        event: delete_dot,
        ..
    } = fixture
        .sender
        .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap()
    else {
        panic!("delete must queue")
    };
    let deleted_item: String =
        sqlx::query_scalar("SELECT item FROM sync_publication_outbox WHERE outbox_id=?")
            .bind(&deleted)
            .fetch_one(&*fixture.author.db)
            .await
            .unwrap();
    let deleted_item: serde_json::Value = serde_json::from_str(&deleted_item).unwrap();
    assert!(
        deleted_item["author_context"].as_str().is_some(),
        "deletion must retain the original observed cut"
    );
    let entry = db::group_db::get_trashed_groups(&fixture.author.db, &fixture.user, 30)
        .await
        .unwrap()
        .into_iter()
        .find(|entry| entry.id == fixture.remote)
        .unwrap();
    let request = db::group_db::GroupRestoreRequest {
        id: entry.id,
        group_global_id: entry.group_global_id,
        deletion_operation_id: entry.deletion_operation_id.unwrap(),
    };
    db::group_db::restore_groups(&fixture.author.db, &fixture.user, &[request], 30)
        .await
        .unwrap();
    let OperationSelection::Queued {
        outbox_id: restored,
        event: restore_dot,
        ..
    } = fixture
        .sender
        .select_next_operation(&fixture.author, &fixture.user, &fixture.author_identity)
        .await
        .unwrap()
    else {
        panic!("restore must queue")
    };
    assert_ne!(delete_dot, restore_dot);
    fixture
        .sender
        .publish_queued(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            &deleted,
        )
        .await
        .unwrap();
    fixture
        .sender
        .publish_queued(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            &restored,
        )
        .await
        .unwrap();
    let (_third_dir, third) = crate::test_support::app_state().await;
    let identity = db::user_db::get_sync_identity(&third.db, fixture.user.id)
        .await
        .unwrap()
        .unwrap();
    let client = AuthenticatedHub::pair(
        &third,
        &fixture.user,
        &identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    for _ in 0..3 {
        client
            .advance_one(&third, &fixture.user, &identity)
            .await
            .unwrap();
    }
    let (group, deleted_at): (i64, Option<String>) = sqlx::query_as("SELECT group_id,group_deleted_at FROM models_group WHERE group_user_id=? AND group_unique_global_id=?").bind(fixture.user.id).bind(&fixture.global).fetch_one(&*third.db).await.unwrap();
    assert_eq!(deleted_at, None);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM models WHERE model_user_id=? AND model_group_id=?"
        )
        .bind(fixture.user.id)
        .bind(group)
        .fetch_one(&*third.db)
        .await
        .unwrap(),
        1
    );
    for event in [delete_dot, restore_dot] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM causal_events WHERE user_id=? AND writer_id=? AND counter=?"
            )
            .bind(fixture.user.id)
            .bind(event.writer_id)
            .bind(event.counter.parse::<i64>().unwrap())
            .fetch_one(&*third.db)
            .await
            .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn own_identity_echo_does_not_count_as_received_name_evidence() {
    let fixture = fixture().await;
    let choice = fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
        .unwrap();
    let linked = fixture
        .client
        .confirm_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .unwrap();
    fixture
        .client
        .publish_queued(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &linked.outbox_id.unwrap(),
        )
        .await
        .unwrap();
    fixture
        .client
        .pull_next(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
        .await
        .unwrap();
    assert!(
        fixture
            .client
            .prepare_received_group_name(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                fixture.local
            )
            .await
            .is_err(),
        "an echoed local observation is not independent received evidence"
    );
}

async fn ambiguous_imported_model(fixture: &Fixture) -> (i64, std::path::PathBuf) {
    let path = fixture.dirs[2].path().join("ambiguous-local.stl");
    std::fs::write(&path, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &fixture.receiver.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        Some(path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let (global, _) = fixture.child.as_ref().unwrap();
    let (name, added, modified): (String, String, String) = sqlx::query_as(
        "SELECT model_name,model_added,model_last_modified FROM models WHERE model_unique_global_id=?",
    ).bind(global).fetch_one(&*fixture.author.db).await.unwrap();
    let model = sqlx::query("INSERT INTO models(model_name,model_blob_id,model_added,model_user_id,model_unique_global_id,model_last_modified,model_group_id,model_birth_id) VALUES(?,?,?,?,?,?,?,NULL)")
        .bind(name).bind(blob).bind(added).bind(fixture.user.id).bind(global).bind(modified).bind(fixture.local)
        .execute(&*fixture.receiver.db).await.unwrap().last_insert_rowid();
    (model, path)
}

async fn model_group_offer(fixture: &Fixture) -> String {
    let AdvanceOutcome::IdentityRequired(choice) = fixture
        .client
        .advance_one(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
        .await
        .unwrap()
    else {
        panic!("the group identity must be offered first")
    };
    assert!(choice.model_identity_required);
    choice.decision_id
}

#[tokio::test]
async fn explicit_same_model_pair_preserves_local_row_and_remote_identity() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, path) = ambiguous_imported_model(&fixture).await;
    let group_decision_id = model_group_offer(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &group_decision_id,
            local,
            GroupIdentityDecision::Same,
        )
        .await
        .expect("a single ambiguous pair must be offered for explicit review");
    assert!(choice.same_model_supported);
    let result = fixture
        .client
        .confirm_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
            ModelIdentityDecision::Same,
        )
        .await
        .unwrap();
    assert_eq!(result.local_model_id, local);
    assert_eq!(result.remote_model_id, local);
    let identity: (String, String) =
        sqlx::query_as("SELECT model_unique_global_id,model_birth_id FROM models WHERE model_id=?")
            .bind(local)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    assert_eq!(identity, fixture.child.unwrap());
    assert_eq!(std::fs::read(path).unwrap(), b"abc");
}

#[tokio::test]
async fn explicit_separate_model_pair_preserves_both_and_is_idempotent() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, path) = ambiguous_imported_model(&fixture).await;
    let group_decision_id = model_group_offer(&fixture).await;
    let original = fixture.child.clone().unwrap();
    let choice = fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &group_decision_id,
            local,
            GroupIdentityDecision::Same,
        )
        .await
        .unwrap();
    let result = fixture
        .client
        .confirm_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
            ModelIdentityDecision::Separate,
        )
        .await
        .unwrap();
    assert_eq!(result.local_model_id, local);
    assert_ne!(result.remote_model_id, local);
    let local_identity: (String, String) =
        sqlx::query_as("SELECT model_unique_global_id,model_birth_id FROM models WHERE model_id=?")
            .bind(local)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    assert_ne!(local_identity.0, original.0);
    let remote_identity: (String, String) =
        sqlx::query_as("SELECT model_unique_global_id,model_birth_id FROM models WHERE model_id=?")
            .bind(result.remote_model_id)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    assert_eq!(remote_identity, original);
    assert_eq!(std::fs::read(path).unwrap(), b"abc");
    let retry = fixture
        .client
        .confirm_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
            ModelIdentityDecision::Separate,
        )
        .await
        .unwrap();
    assert_eq!(
        (retry.local_model_id, retry.remote_model_id),
        (local, result.remote_model_id)
    );
    assert_eq!(retry.group.outbox_id, result.group.outbox_id);
    assert!(
        fixture
            .client
            .confirm_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                ModelIdentityDecision::Same,
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn ambiguous_model_pair_never_implicitly_matches_or_advances_feed() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, path) = ambiguous_imported_model(&fixture).await;
    let old_path_blocked = match fixture
        .client
        .prepare_group_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            fixture.local,
        )
        .await
    {
        Err(_) => true,
        Ok(choice) if !choice.same_group_supported => true,
        Ok(choice) => fixture
            .client
            .confirm_group_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
            )
            .await
            .is_err(),
    };
    assert!(
        old_path_blocked,
        "equal metadata and bytes must not decide identity"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    let birth: Option<String> =
        sqlx::query_scalar("SELECT model_birth_id FROM models WHERE model_id=?")
            .bind(local)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    assert!(birth.is_none());
    assert_eq!(std::fs::read(path).unwrap(), b"abc");
}

#[tokio::test]
async fn same_model_choice_rejects_changed_bytes_without_advancing() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, path) = ambiguous_imported_model(&fixture).await;
    let group_decision_id = model_group_offer(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &group_decision_id,
            local,
            GroupIdentityDecision::Same,
        )
        .await
        .unwrap();
    std::fs::write(&path, b"BAD").unwrap();
    assert!(
        fixture
            .client
            .confirm_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                ModelIdentityDecision::Same,
            )
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    let birth: Option<String> =
        sqlx::query_scalar("SELECT model_birth_id FROM models WHERE model_id=?")
            .bind(local)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    assert!(birth.is_none());
    assert_eq!(std::fs::read(path).unwrap(), b"BAD");
}

#[tokio::test]
async fn model_pair_choice_rejects_changed_local_member_after_offer() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, _) = ambiguous_imported_model(&fixture).await;
    let group_decision_id = model_group_offer(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &group_decision_id,
            local,
            GroupIdentityDecision::Same,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE models SET model_name='changed after offer' WHERE model_id=?")
        .bind(local)
        .execute(&*fixture.receiver.db)
        .await
        .unwrap();
    for decision in [ModelIdentityDecision::Same, ModelIdentityDecision::Separate] {
        assert!(
            fixture
                .client
                .confirm_group_model_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                    decision,
                )
                .await
                .is_err()
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn model_pair_receipt_failure_rolls_back_and_choice_can_retry() {
    for decision in [ModelIdentityDecision::Same, ModelIdentityDecision::Separate] {
        let fixture = fixture_with_remote_model(true).await;
        let (local, path) = ambiguous_imported_model(&fixture).await;
        let group_decision_id = model_group_offer(&fixture).await;
        let choice = fixture
            .client
            .prepare_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &group_decision_id,
                local,
                GroupIdentityDecision::Same,
            )
            .await
            .unwrap();
        sqlx::query("CREATE TRIGGER fail_model_pair_receipt BEFORE INSERT ON sync_pull_receipts BEGIN SELECT RAISE(ABORT,'injected failure'); END")
            .execute(&*fixture.receiver.db).await.unwrap();
        assert!(
            fixture
                .client
                .confirm_group_model_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                    decision,
                )
                .await
                .is_err()
        );
        let identity: (String, Option<String>) = sqlx::query_as(
            "SELECT model_unique_global_id,model_birth_id FROM models WHERE model_id=?",
        )
        .bind(local)
        .fetch_one(&*fixture.receiver.db)
        .await
        .unwrap();
        assert_eq!(identity, (fixture.child.as_ref().unwrap().0.clone(), None));
        assert_eq!(std::fs::read(path).unwrap(), b"abc");
        let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
        let outbox: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_publication_outbox")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
        assert_eq!((receipts, outbox), (0, 0));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models WHERE model_user_id=?")
                .bind(fixture.user.id)
                .fetch_one(&*fixture.receiver.db)
                .await
                .unwrap(),
            1
        );
        sqlx::query("DROP TRIGGER fail_model_pair_receipt")
            .execute(&*fixture.receiver.db)
            .await
            .unwrap();
        assert!(
            fixture
                .client
                .confirm_group_model_identity(
                    &fixture.receiver,
                    &fixture.user,
                    &fixture.receiver_identity,
                    &choice.decision_id,
                    decision,
                )
                .await
                .is_ok()
        );
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Compare each explicit choice across author and third-device relay.
async fn explicit_model_pair_decisions_relay_to_author_and_third_device() {
    for (group_decision, decision, expected_count, expected_groups) in [
        (
            GroupIdentityDecision::Same,
            ModelIdentityDecision::Same,
            1,
            1,
        ),
        (
            GroupIdentityDecision::Same,
            ModelIdentityDecision::Separate,
            2,
            1,
        ),
        (
            GroupIdentityDecision::Separate,
            ModelIdentityDecision::Separate,
            2,
            2,
        ),
    ] {
        let fixture = fixture_with_remote_model(true).await;
        let (local, _) = ambiguous_imported_model(&fixture).await;
        let group_decision_id = model_group_offer(&fixture).await;
        let choice = fixture
            .client
            .prepare_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &group_decision_id,
                local,
                group_decision,
            )
            .await
            .unwrap();
        let outcome = fixture
            .client
            .confirm_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                decision,
            )
            .await
            .unwrap();
        let outbox = outcome
            .group
            .outbox_id
            .expect("pair observation must be durable");
        fixture
            .client
            .publish_queued(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &outbox,
            )
            .await
            .unwrap();
        for _ in 0..2 {
            fixture
                .sender
                .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
                .await
                .unwrap();
        }
        let (_dir, third) = crate::test_support::app_state().await;
        let identity = db::user_db::get_sync_identity(&third.db, fixture.user.id)
            .await
            .unwrap()
            .unwrap();
        let client = AuthenticatedHub::pair(
            &third,
            &fixture.user,
            &identity,
            &fixture.server.origin(),
            &fixture.token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
        )
        .await
        .unwrap();
        for _ in 0..2 {
            client
                .pull_next(&third, &fixture.user, &identity)
                .await
                .unwrap();
        }
        for state in [&fixture.author, &third] {
            let rows: Vec<(String, String)> = sqlx::query_as("SELECT m.model_unique_global_id,b.blob_path FROM models m JOIN blobs b ON b.blob_id=m.model_blob_id WHERE m.model_user_id=? AND m.model_deleted IS NULL")
                .bind(fixture.user.id).fetch_all(&*state.db).await.unwrap();
            assert_eq!(rows.len(), expected_count);
            let groups: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT model_group_id) FROM models WHERE model_user_id=? AND model_deleted IS NULL")
                .bind(fixture.user.id).fetch_one(&*state.db).await.unwrap();
            assert_eq!(groups, expected_groups);
            assert!(
                rows.iter()
                    .any(|(global, _)| *global == fixture.child.as_ref().unwrap().0)
            );
            for (_, path) in rows {
                assert_eq!(std::fs::read(path).unwrap(), b"abc");
            }
        }
    }
}

#[tokio::test]
async fn model_pair_same_with_different_details_requires_version_choice() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, _) = ambiguous_imported_model(&fixture).await;
    sqlx::query("UPDATE models SET model_name='another version' WHERE model_id=?")
        .bind(local)
        .execute(&*fixture.receiver.db)
        .await
        .unwrap();
    let group_decision_id = model_group_offer(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &group_decision_id,
            local,
            GroupIdentityDecision::Same,
        )
        .await
        .unwrap();
    let error = fixture
        .client
        .confirm_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
            ModelIdentityDecision::Same,
        )
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains("model_version_choice_required"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn model_pair_offer_rejects_existing_unknown_tombstone() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, _) = ambiguous_imported_model(&fixture).await;
    let group_decision_id = model_group_offer(&fixture).await;
    sqlx::query(
        "INSERT INTO unknown_model_identity_tombstones(user_id,model_global_id) VALUES(?,?)",
    )
    .bind(fixture.user.id)
    .bind(&fixture.child.as_ref().unwrap().0)
    .execute(&*fixture.receiver.db)
    .await
    .unwrap();
    assert!(
        fixture
            .client
            .prepare_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &group_decision_id,
                local,
                GroupIdentityDecision::Same,
            )
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn advance_one_asks_group_identity_before_ambiguous_model_identity() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, _) = ambiguous_imported_model(&fixture).await;
    let offered = fixture
        .client
        .advance_one(&fixture.receiver, &fixture.user, &fixture.receiver_identity)
        .await
        .unwrap();
    assert!(
        matches!(offered, AdvanceOutcome::IdentityRequired(_)),
        "the group identity must be answered before asking about models"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    let birth: Option<String> =
        sqlx::query_scalar("SELECT model_birth_id FROM models WHERE model_id=?")
            .bind(local)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    assert!(birth.is_none());
}

#[tokio::test]
async fn explicit_separate_groups_keep_separate_models_and_reject_same_model() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, path) = ambiguous_imported_model(&fixture).await;
    let group_decision_id = model_group_offer(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &group_decision_id,
            local,
            GroupIdentityDecision::Separate,
        )
        .await
        .unwrap();
    let error = fixture
        .client
        .confirm_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
            ModelIdentityDecision::Same,
        )
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains("model_membership_choice_required"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    let outcome = fixture
        .client
        .confirm_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
            ModelIdentityDecision::Separate,
        )
        .await
        .unwrap();
    assert_eq!(outcome.group.group_id, fixture.local);
    assert_ne!(outcome.remote_group_id, fixture.local);
    assert_eq!(outcome.local_model_id, local);
    assert_ne!(outcome.remote_model_id, local);
    let local_group: i64 = sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
        .bind(local)
        .fetch_one(&*fixture.receiver.db)
        .await
        .unwrap();
    let remote_group: i64 =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(outcome.remote_model_id)
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    assert_eq!(
        (local_group, remote_group),
        (fixture.local, outcome.remote_group_id)
    );
    assert_eq!(std::fs::read(path).unwrap(), b"abc");
}

#[tokio::test]
async fn model_pair_uses_the_group_offer_without_refetching_feed() {
    let count = Arc::new(AtomicUsize::new(0));
    let fixture = fixture_with_counter(FixtureOptions {
        populated: true,
        counter: Some(count.clone()),
        ..FixtureOptions::default()
    })
    .await;
    let (local, _) = ambiguous_imported_model(&fixture).await;
    count.store(0, Ordering::SeqCst);
    let decision_id = model_group_offer(&fixture).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &decision_id,
            local,
            GroupIdentityDecision::Same,
        )
        .await
        .unwrap();
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "the model choice must use the exact offered delivery"
    );
}

#[tokio::test]
async fn model_pair_requires_group_consent_and_rejects_opposite_retry() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, _) = ambiguous_imported_model(&fixture).await;
    let offer = model_group_offer(&fixture).await;
    assert!(
        fixture
            .client
            .confirm_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &offer,
                ModelIdentityDecision::Separate,
            )
            .await
            .is_err(),
        "a group offer alone is not consent to merge groups"
    );
    let choice = fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &offer,
            local,
            GroupIdentityDecision::Separate,
        )
        .await
        .unwrap();
    assert!(
        fixture
            .client
            .prepare_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &offer,
                local,
                GroupIdentityDecision::Same,
            )
            .await
            .is_err(),
        "the captured group answer cannot silently change"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    assert!(
        fixture
            .client
            .confirm_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                ModelIdentityDecision::Separate,
            )
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn model_pair_rejects_aba_and_foreign_confirmation() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, _) = ambiguous_imported_model(&fixture).await;
    let offer = model_group_offer(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &offer,
            local,
            GroupIdentityDecision::Same,
        )
        .await
        .unwrap();
    let other_id = db::user_db::add_user(
        &fixture.receiver.db,
        "other",
        "pair-foreign@example.com",
        &db::random_hex_32(),
    )
    .await
    .unwrap();
    let other = db::user_db::get_user_by_id(&fixture.receiver.db, other_id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = db::user_db::get_sync_identity(&fixture.receiver.db, other_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        fixture
            .client
            .confirm_group_model_identity(
                &fixture.receiver,
                &other,
                &other_identity,
                &choice.decision_id,
                ModelIdentityDecision::Same,
            )
            .await
            .is_err()
    );
    let original: String = sqlx::query_scalar("SELECT model_name FROM models WHERE model_id=?")
        .bind(local)
        .fetch_one(&*fixture.receiver.db)
        .await
        .unwrap();
    sqlx::query("UPDATE models SET model_name='temporary' WHERE model_id=?")
        .bind(local)
        .execute(&*fixture.receiver.db)
        .await
        .unwrap();
    sqlx::query("UPDATE models SET model_name=? WHERE model_id=?")
        .bind(original)
        .bind(local)
        .execute(&*fixture.receiver.db)
        .await
        .unwrap();
    assert!(
        fixture
            .client
            .confirm_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                ModelIdentityDecision::Same,
            )
            .await
            .is_err(),
        "restoring the visible value must not make a stale offer valid"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn model_pair_cannot_confirm_after_relink() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, _) = ambiguous_imported_model(&fixture).await;
    let offer = model_group_offer(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &offer,
            local,
            GroupIdentityDecision::Same,
        )
        .await
        .unwrap();
    db::user_db::clear_user_sync(&fixture.receiver.db, fixture.user.id)
        .await
        .unwrap();
    let relinked = AuthenticatedHub::pair(
        &fixture.receiver,
        &fixture.user,
        &fixture.receiver_identity,
        &fixture.server.origin(),
        &fixture.token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert!(
        relinked
            .confirm_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                ModelIdentityDecision::Same,
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn model_separation_does_not_certify_later_unknown_incarnations() {
    for delete in [false, true] {
        let fixture = fixture_with_remote_model(true).await;
        let (local, _) = ambiguous_imported_model(&fixture).await;
        let offer = model_group_offer(&fixture).await;
        let choice = fixture
            .client
            .prepare_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &offer,
                local,
                GroupIdentityDecision::Same,
            )
            .await
            .unwrap();
        let outcome = fixture
            .client
            .confirm_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                ModelIdentityDecision::Separate,
            )
            .await
            .unwrap();
        let (writer, counter): (String, i64) = sqlx::query_as("SELECT writer_id,counter FROM causal_genesis_bindings WHERE user_id=? AND entity_kind='model' AND local_id=?")
            .bind(fixture.user.id).bind(outcome.remote_model_id).fetch_one(&*fixture.receiver.db).await.unwrap();
        let dot = db::causal_envelope::Dot {
            writer_id: writer,
            counter: counter.to_string(),
        };
        let state = db::causal_store::get_received(
            &fixture.receiver.db,
            &fixture.user,
            &fixture.receiver_identity,
            &dot,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(state.identity_status, IdentityStatus::Uncontested);
        let later = sqlx::query("INSERT INTO models(model_name,model_blob_id,model_added,model_user_id,model_unique_global_id,model_last_modified,model_group_id,model_birth_id) SELECT model_name,model_blob_id,model_added,model_user_id,model_unique_global_id,model_last_modified,model_group_id,NULL FROM models WHERE model_id=?")
            .bind(outcome.remote_model_id).execute(&*fixture.receiver.db).await.unwrap().last_insert_rowid();
        if delete {
            sqlx::query("DELETE FROM models WHERE model_id=?")
                .bind(later)
                .execute(&*fixture.receiver.db)
                .await
                .unwrap();
        } else {
            sqlx::query("UPDATE models SET model_unique_global_id=? WHERE model_id=?")
                .bind(db::random_hex_32())
                .bind(later)
                .execute(&*fixture.receiver.db)
                .await
                .unwrap();
        }
        let state = db::causal_store::get_received(
            &fixture.receiver.db,
            &fixture.user,
            &fixture.receiver_identity,
            &dot,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            state.identity_status,
            IdentityStatus::Unknown,
            "later independent evidence must not be covered by the first separation certificate"
        );
    }
}

#[tokio::test]
async fn cancel_uncommitted_group_choice_allows_a_fresh_decision() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, path) = ambiguous_imported_model(&fixture).await;
    let offer = model_group_offer(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &offer,
            local,
            GroupIdentityDecision::Separate,
        )
        .await
        .unwrap();
    assert!(
        fixture
            .client
            .confirm_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                ModelIdentityDecision::Same,
            )
            .await
            .is_err()
    );
    fixture
        .client
        .cancel_group_identity_choice(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
        )
        .await
        .expect("the unsupported group choice must be cancelable");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(std::fs::read(path).unwrap(), b"abc");
    let fresh = model_group_offer(&fixture).await;
    assert_ne!(fresh, offer);
    assert!(
        fixture
            .client
            .confirm_group_model_identity(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &choice.decision_id,
                ModelIdentityDecision::Separate,
            )
            .await
            .is_err()
    );
    let next = fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &fresh,
            local,
            GroupIdentityDecision::Same,
        )
        .await
        .unwrap();
    fixture
        .client
        .confirm_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &next.decision_id,
            ModelIdentityDecision::Same,
        )
        .await
        .unwrap();
    assert!(
        fixture
            .client
            .cancel_group_identity_choice(
                &fixture.receiver,
                &fixture.user,
                &fixture.receiver_identity,
                &next.decision_id,
            )
            .await
            .is_err(),
        "a completed decision cannot be canceled"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Exercise a concurrent author move across authenticated relay.
async fn same_model_supplement_does_not_overwrite_authors_later_move() {
    let fixture = fixture_with_remote_model(true).await;
    let (local, _) = ambiguous_imported_model(&fixture).await;
    let offer = model_group_offer(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &offer,
            local,
            GroupIdentityDecision::Same,
        )
        .await
        .unwrap();
    let outcome = fixture
        .client
        .confirm_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
            ModelIdentityDecision::Same,
        )
        .await
        .unwrap();
    let remote_model: i64 = sqlx::query_scalar(
        "SELECT model_id FROM models WHERE model_user_id=? AND model_group_id=?",
    )
    .bind(fixture.user.id)
    .bind(fixture.remote)
    .fetch_one(&*fixture.author.db)
    .await
    .unwrap();
    db::group_db::set_group_id_on_models(
        &fixture.author.db,
        &fixture.user,
        None,
        vec![remote_model],
        None,
    )
    .await
    .unwrap();
    let MoveState::Ready { token, .. } = db::causal_model_move::get_state(
        &fixture.author.db,
        &fixture.user,
        &fixture.author_identity,
        remote_model,
    )
    .await
    .unwrap() else {
        panic!("source move must be available")
    };
    fixture
        .sender
        .publish_operation(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            &token.frontier[0],
        )
        .await
        .unwrap();
    fixture
        .client
        .publish_queued(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &outcome.group.outbox_id.unwrap(),
        )
        .await
        .unwrap();
    for _ in 0..3 {
        fixture
            .sender
            .pull_next(&fixture.author, &fixture.user, &fixture.author_identity)
            .await
            .unwrap();
    }
    let group: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(remote_model)
            .fetch_one(&*fixture.author.db)
            .await
            .unwrap();
    assert_eq!(
        group, None,
        "the independent baseline must not overwrite a later source move"
    );
    let MoveState::Conflict { choices, .. } = db::causal_model_move::get_state(
        &fixture.author.db,
        &fixture.user,
        &fixture.author_identity,
        remote_model,
    )
    .await
    .unwrap() else {
        panic!("independent memberships must remain an explicit conflict")
    };
    assert_eq!(choices.len(), 2);
    assert!(choices.iter().any(|choice| choice.destination.is_none()));
    assert!(choices.iter().any(|choice| {
        choice
            .destination
            .as_ref()
            .is_some_and(|group| group.global_id == fixture.global)
    }));
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Construct a fully authenticated malicious batch from two valid baseline observations.
async fn forged_cross_group_model_baseline_is_rejected_atomically() {
    let digest = |value: &str| {
        sha2::Sha256::digest(value.as_bytes()).iter().fold(
            String::with_capacity(64),
            |mut output, byte| {
                write!(output, "{byte:02x}").unwrap();
                output
            },
        )
    };
    let fixture = fixture_with_remote_model(true).await;
    let (local, _) = ambiguous_imported_model(&fixture).await;
    let offer = model_group_offer(&fixture).await;
    let choice = fixture
        .client
        .prepare_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &offer,
            local,
            GroupIdentityDecision::Same,
        )
        .await
        .unwrap();
    let linked = fixture
        .client
        .confirm_group_model_identity(
            &fixture.receiver,
            &fixture.user,
            &fixture.receiver_identity,
            &choice.decision_id,
            ModelIdentityDecision::Same,
        )
        .await
        .unwrap();
    let item: String =
        sqlx::query_scalar("SELECT item FROM sync_publication_outbox WHERE outbox_id=?")
            .bind(linked.group.outbox_id.unwrap())
            .fetch_one(&*fixture.receiver.db)
            .await
            .unwrap();
    let mut forged: serde_json::Value = serde_json::from_str(&item).unwrap();
    let second = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('other group','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)")
        .bind(fixture.user.id).bind(db::random_hex_32()).execute(&*fixture.author.db).await.unwrap().last_insert_rowid();
    let queued = fixture
        .sender
        .queue_observation(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            second,
        )
        .await
        .unwrap();
    let second_item: String =
        sqlx::query_scalar("SELECT item FROM sync_publication_outbox WHERE outbox_id=?")
            .bind(&queued)
            .fetch_one(&*fixture.author.db)
            .await
            .unwrap();
    fixture
        .sender
        .publish_queued(
            &fixture.author,
            &fixture.user,
            &fixture.author_identity,
            &queued,
        )
        .await
        .unwrap();
    let second_item: serde_json::Value = serde_json::from_str(&second_item).unwrap();
    let second_root =
        parse_causal_envelope(second_item["group"]["envelope"].as_str().unwrap()).unwrap();
    let mut group = parse_causal_envelope(forged["group"]["envelope"].as_str().unwrap()).unwrap();
    let mut model =
        parse_causal_envelope(forged["models"][0]["envelope"].as_str().unwrap()).unwrap();
    let mut group_proof: serde_json::Value =
        serde_json::from_str(forged["group"]["proof"].as_str().unwrap()).unwrap();
    let mut model_proof: serde_json::Value =
        serde_json::from_str(forged["models"][0]["proof"].as_str().unwrap()).unwrap();
    let group_entity = serde_json::to_value(&second_root.entity).unwrap();
    group.entity = second_root.entity.clone();
    group_proof["original"]["global_id"] = second_root.entity.global_id.clone().into();
    let materialization = second_item["group"]["materialization"].as_str().unwrap();
    forged["group"]["materialization"] = materialization.into();
    group_proof["materialization_digest"] = digest(materialization).into();
    model_proof["initial_group"] = group_entity.clone();
    let model_proof = serde_json::to_string(&model_proof).unwrap();
    model.action = Action::LegacyObserved {
        observation_digest: digest(&model_proof),
    };
    forged["models"][0]["proof"] = model_proof.into();
    forged["models"][0]["envelope"] = canonical_causal_envelope(&model).unwrap().into();
    let group_proof = serde_json::to_string(&group_proof).unwrap();
    group.action = Action::LegacyObserved {
        observation_digest: digest(&group_proof),
    };
    forged["group"]["proof"] = group_proof.into();
    forged["group"]["envelope"] = canonical_causal_envelope(&group).unwrap().into();
    let mut manifest: Vec<db::causal_envelope::Dependency> =
        serde_json::from_str(forged["manifest"].as_str().unwrap()).unwrap();
    manifest[1].entity = second_root.entity.clone();
    forged["manifest"] = serde_json::to_string(&manifest).unwrap().into();
    forged["identity_link"]["baseline"] = serde_json::to_value(second_root.event).unwrap();
    db::sync_transport::observations::parse(&forged)
        .expect("the forged batch must pass complete structural validation");
    let proof = db::sync_transport::writer_proof(
        &fixture.receiver.db,
        &fixture.user,
        &fixture.receiver_identity,
        &group.event,
    )
    .await
    .unwrap();
    let (account, secret): (String, String) = sqlx::query_as(
        "SELECT server_account_id,secret FROM sync_transport_local_authorities WHERE user_id=?",
    )
    .bind(fixture.user.id)
    .fetch_one(&*fixture.receiver.db)
    .await
    .unwrap();
    let hub_user = db::user_db::get_user_by_email(&fixture.hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let before: (i64, i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM sync_transport_log),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM causal_event_receipts)").fetch_one(&*fixture.hub.db).await.unwrap();
    let blob: i64 = sqlx::query_scalar(
        "SELECT blob_id FROM blobs WHERE blob_sha256='ba7816bf8f01cfea414140de5dae2223'",
    )
    .fetch_one(&*fixture.hub.db)
    .await
    .unwrap();
    let transaction = db::sync_transport::observations::begin(
        &fixture.hub.db,
        &hub_user,
        &account,
        &proof.connection_id,
        &secret,
        Some(&proof.writer_nonce),
        forged,
    )
    .await
    .expect("batch and author must authenticate");
    let rejected = transaction
        .commit(vec![("ba7816bf8f01cfea414140de5dae2223".into(), blob)])
        .await;
    assert!(
        rejected.is_err(),
        "a model baseline from another group must not authenticate the group match"
    );
    let after: (i64, i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM sync_transport_log),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM causal_event_receipts)").fetch_one(&*fixture.hub.db).await.unwrap();
    assert_eq!(before, after);
}
