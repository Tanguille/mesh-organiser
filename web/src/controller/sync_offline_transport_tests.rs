//! Offline authority is private and does not enable the production sync route.
use super::*;

#[path = "sync_combined_transport_tests.rs"]
mod combined;
#[path = "sync_observation_transport_tests.rs"]
mod observations;

fn publish_offline(
    authority: &db::sync_transport::StoredAuthority,
    proof: &db::sync_transport::OfflineWriterProof,
    item: &serde_json::Value,
    cookie: &str,
) -> Request<Body> {
    let body = serde_json::json!({"protocol":1,"connection_id":proof.connection_id,"writer_nonce":proof.writer_nonce,"item":item});
    post(
        "/api/v2/sync/items",
        &body,
        Some(cookie),
        Some(authority.writer_secret()),
    )
}

async fn register_identity(
    router: &Router,
    cookie: &str,
    identity: &db::user_db::SyncIdentity,
) -> db::sync_transport::WriterRegistration {
    let claim = serde_json::json!({"protocol":1,"client_identity":identity});
    let response = router
        .clone()
        .oneshot(post("/api/v2/sync/handshake", &claim, Some(cookie), None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap()
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn offline_edit_after_restart_uses_fresh_authorized_writer_and_keeps_old_dot() {
    use db::causal_group_rename::NameState;
    let (_hub_dir, hub, router, cookie) = app().await;
    let (source_dir, source) = crate::test_support::app_state().await;
    let registration = registration(&router, &cookie, &source).await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    db::sync_peer::bind_peer(&source.db, &user, &identity, &registration.server_identity)
        .await
        .unwrap();
    let installed =
        db::sync_transport::install_authority(&source.db, &user, &identity, &registration).await;
    assert!(
        installed.is_ok(),
        "an authenticated pinned connection must support offline authority: {installed:?}"
    );
    let authority = db::sync_transport::load_authority(
        &source.db,
        &user,
        &identity,
        &registration.connection_id,
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
    let original = root.event().clone();
    let proof = db::sync_transport::writer_proof(&source.db, &user, &identity, &original)
        .await
        .unwrap();
    assert!(!format!("{authority:?}").contains(authority.writer_secret()));
    assert!(
        !serde_json::to_string(&proof)
            .unwrap()
            .contains(authority.writer_secret())
    );
    let before_retry: (String, i64) =
        sqlx::query_as("SELECT writer_id,counter FROM causal_account_writers WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &registration)
        .await
        .unwrap();
    let after_retry: (String, i64) =
        sqlx::query_as("SELECT writer_id,counter FROM causal_account_writers WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    assert_eq!(
        after_retry, before_retry,
        "exact authority retry must not rotate or reset the clock"
    );
    let genesis = db::sync_transport::encode_genesis(&root).unwrap();
    let mut forged = proof.clone();
    forged.writer_nonce = format!("{}{}", db::random_hex_32(), db::random_hex_32());
    let before: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log)",
    )
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    let denied = router
        .clone()
        .oneshot(publish_offline(&authority, &forged, &genesis, &cookie))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let after: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log)",
    )
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(
        after, before,
        "a forged nonce must not write a receipt or event"
    );
    let delegation_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_transport_delegations")
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    sqlx::query("CREATE TRIGGER reject_offline_log BEFORE INSERT ON sync_transport_log BEGIN SELECT RAISE(ABORT,'injected log failure'); END").execute(&*hub.db).await.unwrap();
    let rejected = router
        .clone()
        .oneshot(publish_offline(&authority, &proof, &genesis, &cookie))
        .await
        .unwrap();
    assert_ne!(rejected.status(), StatusCode::OK);
    let remaining: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log),(SELECT COUNT(*) FROM sync_transport_delegations)").fetch_one(&*hub.db).await.unwrap();
    assert_eq!(remaining, (before.0, before.1, delegation_count));
    sqlx::query("DROP TRIGGER reject_offline_log")
        .execute(&*hub.db)
        .await
        .unwrap();
    let response = router
        .clone()
        .oneshot(publish_offline(&authority, &proof, &genesis, &cookie))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let reopened = db::db_context::setup_db(
        &source_dir.path().join("db.sqlite"),
        &source_dir.path().join("backups"),
    )
    .await;
    db::group_db::edit_group(&reopened, &user, group.id, "offline edit", None, None)
        .await
        .unwrap();
    let state = db::causal_group_lifecycle::get_state(&reopened, &user, &identity, group.id)
        .await
        .unwrap();
    let dot = match state.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        other => panic!("expected ready rename: {other:?}"),
    };
    assert_ne!(
        dot.writer_id, original.writer_id,
        "restart must use a fresh writer"
    );
    let next_proof = db::sync_transport::writer_proof(&reopened, &user, &identity, &dot)
        .await
        .unwrap();
    assert_ne!(next_proof.writer_nonce, proof.writer_nonce);
    let operation = db::causal_group_lifecycle::export_operation(&reopened, &user, &identity, &dot)
        .await
        .unwrap();
    let item = db::sync_transport::encode_operation(&operation).unwrap();
    let authority = db::sync_transport::load_authority(
        &reopened,
        &user,
        &identity,
        &registration.connection_id,
    )
    .await
    .unwrap();
    let response = router
        .oneshot(publish_offline(&authority, &next_proof, &item, &cookie))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a genuine offline edit must publish after restart"
    );
    let owner = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let name: String = sqlx::query_scalar(
        "SELECT group_name FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(owner.id)
    .bind(&root.entity().global_id)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(name, "offline edit");
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM causal_events WHERE writer_id=? AND counter=?")
            .bind(&original.writer_id)
            .bind(original.counter.parse::<i64>().unwrap())
            .fetch_one(&reopened)
            .await
            .unwrap();
    assert_eq!(count, 1, "reopening must retain the original root dot");
}

#[tokio::test]
async fn separate_accounts_keep_separate_authority_clocks_and_proofs() {
    let (_hub_dir, _hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let first = db::model::user::User::default();
    let second_id = db::user_db::add_user(
        &source.db,
        "second",
        "offline-second@example.test",
        &db::random_hex_32(),
    )
    .await
    .unwrap();
    let second = db::user_db::get_user_by_id(&source.db, second_id)
        .await
        .unwrap()
        .unwrap();
    let first_identity = db::user_db::get_sync_identity(&source.db, first.id)
        .await
        .unwrap()
        .unwrap();
    let second_identity = db::user_db::get_sync_identity(&source.db, second.id)
        .await
        .unwrap()
        .unwrap();
    let registrations = [
        register_identity(&router, &cookie, &first_identity).await,
        register_identity(&router, &cookie, &second_identity).await,
    ];
    for (user, identity, registration) in [
        (&first, &first_identity, &registrations[0]),
        (&second, &second_identity, &registrations[1]),
    ] {
        db::sync_peer::bind_peer(&source.db, user, identity, &registration.server_identity)
            .await
            .unwrap();
        db::sync_transport::install_authority(&source.db, user, identity, registration)
            .await
            .unwrap();
    }
    let first_group = db::group_db::add_empty_group(&source.db, &first, "first", None)
        .await
        .unwrap();
    let second_group = db::group_db::add_empty_group(&source.db, &second, "second", None).await;
    assert!(
        second_group.is_ok(),
        "a second account must allocate from its own writer"
    );
    let second_group = second_group.unwrap();
    let first_root = db::causal_group_lifecycle::enroll_pristine_group(
        &source.db,
        &first,
        &first_identity,
        first_group.id,
    )
    .await
    .unwrap();
    let second_root = db::causal_group_lifecycle::enroll_pristine_group(
        &source.db,
        &second,
        &second_identity,
        second_group.id,
    )
    .await
    .unwrap();
    assert_ne!(first_root.event().writer_id, second_root.event().writer_id);
    let first_proof =
        db::sync_transport::writer_proof(&source.db, &first, &first_identity, first_root.event())
            .await
            .unwrap();
    let second_proof = db::sync_transport::writer_proof(
        &source.db,
        &second,
        &second_identity,
        second_root.event(),
    )
    .await
    .unwrap();
    assert_eq!(first_proof.connection_id, registrations[0].connection_id);
    assert_eq!(second_proof.connection_id, registrations[1].connection_id);
    assert!(
        db::sync_transport::writer_proof(&source.db, &first, &first_identity, second_root.event())
            .await
            .is_err()
    );
    assert!(
        db::sync_transport::writer_proof(&source.db, &second, &second_identity, first_root.event())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn pre_authority_event_cannot_gain_a_fabricated_proof() {
    let (_hub_dir, _hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = db::group_db::add_empty_group(&source.db, &user, "old", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    let original = root.event().clone();
    let before: String =
        sqlx::query_scalar("SELECT envelope FROM causal_events WHERE writer_id=? AND counter=?")
            .bind(&original.writer_id)
            .bind(original.counter.parse::<i64>().unwrap())
            .fetch_one(&*source.db)
            .await
            .unwrap();
    let registration = registration(&router, &cookie, &source).await;
    db::sync_peer::bind_peer(&source.db, &user, &identity, &registration.server_identity)
        .await
        .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &registration)
        .await
        .unwrap();
    assert!(
        db::sync_transport::writer_proof(&source.db, &user, &identity, &original)
            .await
            .is_err()
    );
    let after: String =
        sqlx::query_scalar("SELECT envelope FROM causal_events WHERE writer_id=? AND counter=?")
            .bind(&original.writer_id)
            .bind(original.counter.parse::<i64>().unwrap())
            .fetch_one(&*source.db)
            .await
            .unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn independently_reopened_database_copies_get_distinct_offline_writers() {
    use db::causal_group_rename::NameState;
    let (_hub_dir, hub, router, cookie) = app().await;
    let (source_dir, source) = crate::test_support::app_state().await;
    let registration = registration(&router, &cookie, &source).await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    db::sync_peer::bind_peer(&source.db, &user, &identity, &registration.server_identity)
        .await
        .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &registration)
        .await
        .unwrap();
    let authority = db::sync_transport::load_authority(
        &source.db,
        &user,
        &identity,
        &registration.connection_id,
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
    let proof = db::sync_transport::writer_proof(&source.db, &user, &identity, root.event())
        .await
        .unwrap();
    let item = db::sync_transport::encode_genesis(&root).unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(publish_offline(&authority, &proof, &item, &cookie))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let copied_path = source_dir.path().join("copy.sqlite");
    sqlx::query("VACUUM INTO ?")
        .bind(copied_path.to_str().unwrap())
        .execute(&*source.db)
        .await
        .unwrap();
    let first = db::db_context::setup_db(
        &source_dir.path().join("db.sqlite"),
        &source_dir.path().join("backups"),
    )
    .await;
    let second = db::db_context::setup_db(&copied_path, &source_dir.path().join("backups")).await;
    let mut dots = Vec::new();
    for (db, name) in [(&first, "alpha"), (&second, "bravo")] {
        db::group_db::edit_group(db, &user, group.id, name, None, None)
            .await
            .unwrap();
        let state = db::causal_group_lifecycle::get_state(db, &user, &identity, group.id)
            .await
            .unwrap();
        let dot = match state.name {
            NameState::Ready { token, .. } => token.frontier[0].clone(),
            other => panic!("expected ready name: {other:?}"),
        };
        let event_proof = db::sync_transport::writer_proof(db, &user, &identity, &dot)
            .await
            .unwrap();
        let operation = db::causal_group_lifecycle::export_operation(db, &user, &identity, &dot)
            .await
            .unwrap();
        let item = db::sync_transport::encode_operation(&operation).unwrap();
        let response = router
            .clone()
            .oneshot(publish_offline(&authority, &event_proof, &item, &cookie))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        dots.push(dot);
    }
    assert_ne!(
        dots[0].writer_id, dots[1].writer_id,
        "copied databases must derive independent writers"
    );
    let owner = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let hub_identity = db::user_db::get_sync_identity(&hub.db, owner.id)
        .await
        .unwrap()
        .unwrap();
    let hub_group: i64 = sqlx::query_scalar(
        "SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(owner.id)
    .bind(&root.entity().global_id)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    let state = db::causal_group_lifecycle::get_state(&hub.db, &owner, &hub_identity, hub_group)
        .await
        .unwrap();
    match state.name {
        NameState::Conflict { choices, .. } => {
            assert!(choices.iter().any(|choice| choice.name == "alpha"));
            assert!(choices.iter().any(|choice| choice.name == "bravo"));
        }
        other => panic!("both offline branches must remain choices: {other:?}"),
    }
}

#[tokio::test]
async fn two_open_pools_on_one_file_allocate_distinct_offline_dots() {
    let (_hub_dir, _hub, router, cookie) = app().await;
    let (source_dir, source) = crate::test_support::app_state().await;
    let registration = registration(&router, &cookie, &source).await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    db::sync_peer::bind_peer(&source.db, &user, &identity, &registration.server_identity)
        .await
        .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &registration)
        .await
        .unwrap();
    let second = db::db_context::setup_db(
        &source_dir.path().join("db.sqlite"),
        &source_dir.path().join("backups"),
    )
    .await;
    let first_task = async {
        let group = db::group_db::add_empty_group(&source.db, &user, "one", None)
            .await
            .unwrap();
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap()
    };
    let second_task = async {
        let group = db::group_db::add_empty_group(&second, &user, "two", None)
            .await
            .unwrap();
        db::causal_group_lifecycle::enroll_pristine_group(&second, &user, &identity, group.id)
            .await
            .unwrap()
    };
    let (first, second_root) = tokio::join!(first_task, second_task);
    assert_ne!(first.event(), second_root.event());
    let first_proof = db::sync_transport::writer_proof(&source.db, &user, &identity, first.event())
        .await
        .unwrap();
    let second_proof =
        db::sync_transport::writer_proof(&second, &user, &identity, second_root.event())
            .await
            .unwrap();
    assert_eq!(first_proof.connection_id, registration.connection_id);
    assert_eq!(second_proof.connection_id, registration.connection_id);
}

#[tokio::test]
async fn malformed_or_unpinned_authority_cannot_change_account_clock() {
    let (_hub_dir, _hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let registration = registration(&router, &cookie, &source).await;
    db::sync_peer::bind_peer(&source.db, &user, &identity, &registration.server_identity)
        .await
        .unwrap();
    let before: (i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM sync_transport_local_authorities WHERE user_id=?),(SELECT COUNT(*) FROM causal_account_writers WHERE user_id=?)").bind(user.id).bind(user.id).fetch_one(&*source.db).await.unwrap();
    for field in ["secret", "connection", "writer", "server"] {
        let mut forged = registration.clone();
        match field {
            "secret" => forged.writer_secret = "z".repeat(64),
            "connection" => forged.connection_id = "z".repeat(32),
            "writer" => forged.writer_id = "z".repeat(32),
            "server" => forged.server_identity.account_id = db::random_hex_32(),
            _ => unreachable!(),
        }
        assert!(
            db::sync_transport::install_authority(&source.db, &user, &identity, &forged)
                .await
                .is_err()
        );
        let count: (i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM sync_transport_local_authorities WHERE user_id=?),(SELECT COUNT(*) FROM causal_account_writers WHERE user_id=?)").bind(user.id).bind(user.id).fetch_one(&*source.db).await.unwrap();
        assert_eq!(
            count, before,
            "invalid {field} must not write authority or clock"
        );
    }
    assert!(
        db::sync_transport::install_authority(&source.db, &user, &identity, &registration)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn legacy_activation_does_not_give_other_accounts_the_issued_writer() {
    use db::causal_group_rename::NameState;
    let (_hub_dir, _hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let alice = db::model::user::User::default();
    let bob_id = db::user_db::add_user(
        &source.db,
        "bob",
        "fallback-bob@example.test",
        &db::random_hex_32(),
    )
    .await
    .unwrap();
    let bob = db::user_db::get_user_by_id(&source.db, bob_id)
        .await
        .unwrap()
        .unwrap();
    let ai = db::user_db::get_sync_identity(&source.db, alice.id)
        .await
        .unwrap()
        .unwrap();
    let bi = db::user_db::get_sync_identity(&source.db, bob.id)
        .await
        .unwrap()
        .unwrap();
    let registration = registration(&router, &cookie, &source).await;
    db::sync_peer::bind_peer(&source.db, &alice, &ai, &registration.server_identity)
        .await
        .unwrap();
    db::sync_transport::activate_writer(&source.db, &alice, &ai, &registration)
        .await
        .unwrap();
    let alice_group = db::group_db::add_empty_group(&source.db, &alice, "alice", None)
        .await
        .unwrap();
    let alice_root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &alice, &ai, alice_group.id)
            .await
            .unwrap();
    assert_eq!(alice_root.event().writer_id, registration.writer_id);
    let bob_group = db::group_db::add_empty_group(&source.db, &bob, "bob", None)
        .await
        .unwrap();
    let bob_root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &bob, &bi, bob_group.id)
            .await
            .unwrap();
    assert_ne!(
        bob_root.event().writer_id,
        registration.writer_id,
        "a different unlinked account must never author under Alice's issued writer"
    );
    db::group_db::edit_group(&source.db, &bob, bob_group.id, "bob edit", None, None)
        .await
        .unwrap();
    let state = db::causal_group_lifecycle::get_state(&source.db, &bob, &bi, bob_group.id)
        .await
        .unwrap();
    let dot = match state.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        other => panic!("expected ready rename: {other:?}"),
    };
    assert_ne!(dot.writer_id, registration.writer_id);
}

#[tokio::test]
async fn unlink_removes_active_authority_but_retains_existing_event_evidence() {
    let (_hub_dir, _hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let registration = registration(&router, &cookie, &source).await;
    db::sync_peer::bind_peer(&source.db, &user, &identity, &registration.server_identity)
        .await
        .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &registration)
        .await
        .unwrap();
    let group = db::group_db::add_empty_group(&source.db, &user, "before unlink", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    let dot = root.event().clone();
    let event: String =
        sqlx::query_scalar("SELECT envelope FROM causal_events WHERE writer_id=? AND counter=?")
            .bind(&dot.writer_id)
            .bind(dot.counter.parse::<i64>().unwrap())
            .fetch_one(&*source.db)
            .await
            .unwrap();
    let delegation: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM local_writer_delegations WHERE user_id=? AND writer_id=?",
    )
    .bind(user.id)
    .bind(&dot.writer_id)
    .fetch_one(&*source.db)
    .await
    .unwrap();
    assert_eq!(delegation, 1);
    db::user_db::clear_user_sync(&source.db, user.id)
        .await
        .unwrap();
    assert!(
        db::sync_transport::load_authority(
            &source.db,
            &user,
            &identity,
            &registration.connection_id
        )
        .await
        .is_err(),
        "unlink must remove the active secret"
    );
    let active: (i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM sync_transport_local_authorities WHERE user_id=?),(SELECT COUNT(*) FROM causal_account_writers WHERE user_id=?)").bind(user.id).bind(user.id).fetch_one(&*source.db).await.unwrap();
    assert_eq!(active, (0, 0));
    let retained: (String,i64) = sqlx::query_as("SELECT envelope,(SELECT COUNT(*) FROM local_writer_delegations WHERE user_id=? AND writer_id=?) FROM causal_events WHERE writer_id=? AND counter=?").bind(user.id).bind(&dot.writer_id).bind(&dot.writer_id).bind(dot.counter.parse::<i64>().unwrap()).fetch_one(&*source.db).await.unwrap();
    assert_eq!(retained, (event, delegation));
}

#[tokio::test]
async fn deleted_source_account_cannot_leak_authority_to_reused_numeric_id() {
    let (_hub_dir, _hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let user_id = db::user_db::add_user(
        &source.db,
        "owner",
        "offline-owner@example.test",
        &db::random_hex_32(),
    )
    .await
    .unwrap();
    let user = db::user_db::get_user_by_id(&source.db, user_id)
        .await
        .unwrap()
        .unwrap();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let claim = serde_json::json!({"protocol":1,"client_identity":identity});
    let response = router
        .oneshot(post("/api/v2/sync/handshake", &claim, Some(&cookie), None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let registration: db::sync_transport::WriterRegistration =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    db::sync_peer::bind_peer(&source.db, &user, &identity, &registration.server_identity)
        .await
        .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &registration)
        .await
        .unwrap();
    sqlx::query("DELETE FROM users WHERE user_id=?")
        .bind(user.id)
        .execute(&*source.db)
        .await
        .unwrap();
    let secret_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_transport_local_authorities WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*source.db)
            .await
            .unwrap();
    assert_eq!(secret_count, 0);
    let hash = db::model::user::hash_password(&db::random_hex_32());
    sqlx::query("INSERT INTO users (user_id,user_name,user_email,user_password_hash,user_created_at) VALUES (?,'replacement','offline-replacement@example.test',?,'2020-01-01T00:00:00Z')").bind(user.id).bind(hash).execute(&*source.db).await.unwrap();
    let replacement = db::user_db::get_user_by_id(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let replacement_identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(replacement_identity.account_id, identity.account_id);
    assert!(
        db::sync_transport::load_authority(
            &source.db,
            &replacement,
            &replacement_identity,
            &registration.connection_id
        )
        .await
        .is_err()
    );
    assert!(
        db::sync_transport::load_authority(
            &source.db,
            &replacement,
            &identity,
            &registration.connection_id
        )
        .await
        .is_err()
    );
}
