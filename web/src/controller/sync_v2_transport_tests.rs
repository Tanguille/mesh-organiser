//! Opt-in internal transport exercises real login sessions, not public sync availability.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use tower::ServiceExt;

#[path = "sync_identity_choice_tests.rs"]
mod identity_choice;
#[path = "sync_outbox_transport_tests.rs"]
mod outbox;
#[path = "sync_pair_transport_tests.rs"]
mod pair;
#[path = "sync_pull_transport_tests.rs"]
mod pull;
#[path = "sync_push_transport_tests.rs"]
mod push;

async fn app() -> (tempfile::TempDir, service::AppState, Router, String) {
    let (dir, state) = crate::test_support::app_state().await;
    let password = crate::test_support::add_login_user(&state).await;
    let router = crate::test_support::with_session(
        crate::controller::auth_controller::login_router().merge(super::router()),
        state.clone(),
    )
    .await;
    let login = router
        .clone()
        .oneshot(crate::test_support::login_request(&password))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::NO_CONTENT);
    let cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    (dir, state, router, cookie)
}

fn post(
    path: &str,
    body: &serde_json::Value,
    cookie: Option<&str>,
    secret: Option<&str>,
) -> Request<Body> {
    let mut req = Request::post(path).header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        req = req.header(header::COOKIE, cookie);
    }
    if let Some(secret) = secret {
        req = req.header("x-sync-writer-secret", secret);
    }
    req.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn handshake_requires_real_session_and_allocates_distinct_writer_credentials() {
    let (_server_dir, _server, router, cookie) = app().await;
    let (_client_dir, client) = crate::test_support::app_state().await;
    let identity = db::user_db::get_sync_identity(&client.db, 1)
        .await
        .unwrap()
        .unwrap();
    let request = serde_json::json!({"protocol":1,"client_identity":identity});
    let mut responses = Vec::new();
    for _ in 0..2 {
        let response = router
            .clone()
            .oneshot(post(
                "/api/v2/sync/handshake",
                &request,
                Some(&cookie),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "an authenticated current client must receive a writer registration"
        );
        responses.push(
            serde_json::from_slice::<serde_json::Value>(
                &to_bytes(response.into_body(), 8192).await.unwrap(),
            )
            .unwrap(),
        );
    }
    for field in ["connection_id", "writer_id", "writer_secret"] {
        assert!(responses[0][field].as_str().is_some_and(|s| !s.is_empty()));
        assert_ne!(
            responses[0][field], responses[1][field],
            "each connection receives its own {field}"
        );
    }
    let anonymous = router
        .clone()
        .oneshot(post("/api/v2/sync/handshake", &request, None, None))
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let unsupported = serde_json::json!({"protocol":0,"client_identity":identity});
    let response = router
        .clone()
        .oneshot(post(
            "/api/v2/sync/handshake",
            &unsupported,
            Some(&cookie),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
    assert_eq!(responses[0]["writer_secret"].as_str().unwrap().len(), 64);
}

#[tokio::test]
async fn old_password_client_must_upgrade_before_obtaining_a_session() {
    let (_dir, state) = crate::test_support::app_state().await;
    let password = crate::test_support::add_login_user(&state).await;
    let router = crate::test_support::with_session(
        crate::controller::auth_controller::login_router().merge(super::router()),
        state,
    )
    .await;
    let request = serde_json::json!({"email":"alice@example.com","password":password});
    let response = router
        .oneshot(post("/api/v1/login/password", &request, None, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
    assert!(!response.headers().contains_key(header::SET_COOKIE));
}

async fn registration(
    router: &Router,
    cookie: &str,
    client: &service::AppState,
) -> db::sync_transport::WriterRegistration {
    let identity = db::user_db::get_sync_identity(&client.db, 1)
        .await
        .unwrap()
        .unwrap();
    let request = serde_json::json!({"protocol":1,"client_identity":identity});
    let response = router
        .clone()
        .oneshot(post("/api/v2/sync/handshake", &request, Some(cookie), None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap()
}

fn publish(
    registration: &db::sync_transport::WriterRegistration,
    item: &serde_json::Value,
    cookie: &str,
    secret: &str,
) -> Request<Body> {
    let body =
        serde_json::json!({"protocol":1,"connection_id":registration.connection_id,"item":item});
    post("/api/v2/sync/items", &body, Some(cookie), Some(secret))
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn real_writer_events_are_owner_bound_replayable_and_relayed_without_reauthoring() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_a_dir, a) = crate::test_support::app_state().await;
    let (_b_dir, b) = crate::test_support::app_state().await;
    let (_c_dir, c) = crate::test_support::app_state().await;
    let a_registration = registration(&router, &cookie, &a).await;
    let b_registration = registration(&router, &cookie, &b).await;
    let c_registration = registration(&router, &cookie, &c).await;
    let oversized = serde_json::json!({"kind":"group_genesis","root":"x".repeat(1024 * 1024 + 1),"materialization":{"name":"n","created":"2020-01-01T00:00:00Z","last_modified":"2020-01-01T00:00:00Z"}});
    let response = router
        .clone()
        .oneshot(publish(
            &a_registration,
            &oversized,
            &cookie,
            &a_registration.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let user = db::model::user::User::default();
    let a_identity = db::user_db::get_sync_identity(&a.db, user.id)
        .await
        .unwrap()
        .unwrap();
    db::sync_peer::bind_peer(&a.db, &user, &a_identity, &a_registration.server_identity)
        .await
        .unwrap();
    db::sync_transport::activate_writer(&a.db, &user, &a_identity, &a_registration)
        .await
        .unwrap();
    let group = db::group_db::add_empty_group(&a.db, &user, "source", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&a.db, &user, &a_identity, group.id)
            .await
            .unwrap();
    assert_eq!(root.event().writer_id, a_registration.writer_id);
    let item = db::sync_transport::encode_genesis(&root).unwrap();
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events")
        .fetch_one(&*hub.db)
        .await
        .unwrap();
    let forged = router
        .clone()
        .oneshot(publish(
            &b_registration,
            &item,
            &cookie,
            &b_registration.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(
        forged.status(),
        StatusCode::FORBIDDEN,
        "a registered device cannot introduce another writer's event"
    );
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events")
        .fetch_one(&*hub.db)
        .await
        .unwrap();
    assert_eq!(after, before);
    let bad = router
        .clone()
        .oneshot(publish(
            &a_registration,
            &item,
            &cookie,
            &b_registration.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(
        bad.status(),
        StatusCode::FORBIDDEN,
        "another device must not publish under the source writer"
    );
    let accepted = router
        .clone()
        .oneshot(publish(
            &a_registration,
            &item,
            &cookie,
            &a_registration.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(accepted.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(body["status"], "applied");
    let replay = router
        .clone()
        .oneshot(publish(
            &b_registration,
            &item,
            &cookie,
            &b_registration.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body: serde_json::Value =
        serde_json::from_slice(&to_bytes(replay.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(replay_body["status"], "already_present");
    assert_eq!(replay_body["sequence"], body["sequence"]);
    let path = format!(
        "/api/v2/sync/items?protocol=1&connection_id={}&after=0&limit=1",
        c_registration.connection_id
    );
    let request = Request::get(path)
        .header(header::COOKIE, &cookie)
        .header("x-sync-writer-secret", &c_registration.writer_secret)
        .body(Body::empty())
        .unwrap();
    let pulled = router.clone().oneshot(request).await.unwrap();
    assert_eq!(pulled.status(), StatusCode::OK);
    let page: serde_json::Value =
        serde_json::from_slice(&to_bytes(pulled.into_body(), 2 * 1024 * 1024).await.unwrap())
            .unwrap();
    assert!(
        !page.to_string().contains(&a_registration.writer_secret),
        "writer credentials must never appear in a read page"
    );
    assert_eq!(
        page["items"][0]["item"], item,
        "relay must preserve the exact original authored event"
    );
    let through = page["through"].as_str().unwrap();
    let next = page["next_after"].as_str().unwrap();
    let later_group = db::group_db::add_empty_group(&a.db, &user, "later", None)
        .await
        .unwrap();
    let later_root = db::causal_group_lifecycle::enroll_pristine_group(
        &a.db,
        &user,
        &a_identity,
        later_group.id,
    )
    .await
    .unwrap();
    let later_item = db::sync_transport::encode_genesis(&later_root).unwrap();
    let published = router
        .clone()
        .oneshot(publish(
            &a_registration,
            &later_item,
            &cookie,
            &a_registration.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(published.status(), StatusCode::OK);
    let path = format!(
        "/api/v2/sync/items?protocol=1&connection_id={}&after={next}&limit=1&through={through}",
        c_registration.connection_id
    );
    let request = Request::get(path)
        .header(header::COOKIE, &cookie)
        .header("x-sync-writer-secret", &c_registration.writer_secret)
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bounded: serde_json::Value = serde_json::from_slice(
        &to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        bounded["items"].as_array().unwrap().is_empty(),
        "new events must not enter an existing through bound"
    );
    let path = format!(
        "/api/v2/sync/items?protocol=1&connection_id={}&after={next}&limit=1",
        c_registration.connection_id
    );
    let request = Request::get(path)
        .header(header::COOKIE, &cookie)
        .header("x-sync-writer-secret", &c_registration.writer_secret)
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let next_page: serde_json::Value = serde_json::from_slice(
        &to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(next_page["items"][0]["item"], later_item);
}

#[tokio::test]
async fn handshake_rejects_session_after_numeric_account_id_is_reused() {
    let (_dir, state, router, cookie) = app().await;
    let user = db::user_db::get_user_by_email(&state.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let password_hash: String =
        sqlx::query_scalar("SELECT user_password_hash FROM users WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*state.db)
            .await
            .unwrap();
    sqlx::query("DELETE FROM users WHERE user_id=?")
        .bind(user.id)
        .execute(&*state.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (user_id,user_name,user_email,user_password_hash,user_created_at) VALUES (?,'replacement','replacement@example.com',?,'2020-01-01T00:00:00Z')").bind(user.id).bind(password_hash).execute(&*state.db).await.unwrap();
    let (_client_dir, client) = crate::test_support::app_state().await;
    let identity = db::user_db::get_sync_identity(&client.db, 1)
        .await
        .unwrap()
        .unwrap();
    let body = serde_json::json!({"protocol":1,"client_identity":identity});
    let response = router
        .oneshot(post("/api/v2/sync/handshake", &body, Some(&cookie), None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn occupied_populated_target_is_rejected_without_partial_writes() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let registration = registration(&router, &cookie, &source).await;
    let source_user = db::model::user::User::default();
    let source_identity = db::user_db::get_sync_identity(&source.db, source_user.id)
        .await
        .unwrap()
        .unwrap();
    db::sync_peer::bind_peer(
        &source.db,
        &source_user,
        &source_identity,
        &registration.server_identity,
    )
    .await
    .unwrap();
    db::sync_transport::activate_writer(&source.db, &source_user, &source_identity, &registration)
        .await
        .unwrap();
    let source_group = db::group_db::add_empty_group(&source.db, &source_user, "source", None)
        .await
        .unwrap();
    let root = db::causal_group_lifecycle::enroll_pristine_group(
        &source.db,
        &source_user,
        &source_identity,
        source_group.id,
    )
    .await
    .unwrap();
    let item = db::sync_transport::encode_genesis(&root).unwrap();
    let owner = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let occupied = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES ('existing','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(owner.id).bind(&root.entity().global_id).execute(&*hub.db).await.unwrap().last_insert_rowid();
    let blob = db::blob_db::add_blob(&hub.db, &db::random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let model = db::model_db::add_model(&hub.db, &owner, "existing model", blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&hub.db, &owner, Some(occupied), vec![model], None)
        .await
        .unwrap();
    let before: (String,Option<i64>,i64,i64) = sqlx::query_as("SELECT g.group_name,m.model_group_id,(SELECT COUNT(*) FROM causal_events),(SELECT counter FROM group_causal_replica WHERE singleton=1) FROM models_group g JOIN models m ON m.model_id=? WHERE g.group_id=?").bind(model).bind(occupied).fetch_one(&*hub.db).await.unwrap();
    let response = router
        .oneshot(publish(
            &registration,
            &item,
            &cookie,
            &registration.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let after: (String,Option<i64>,i64,i64) = sqlx::query_as("SELECT g.group_name,m.model_group_id,(SELECT COUNT(*) FROM causal_events),(SELECT counter FROM group_causal_replica WHERE singleton=1) FROM models_group g JOIN models m ON m.model_id=? WHERE g.group_id=?").bind(model).bind(occupied).fetch_one(&*hub.db).await.unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn authenticated_other_account_cannot_list_or_replay_an_owners_events() {
    let (_dir, hub, router, alice_cookie) = app().await;
    let (_a_dir, a) = crate::test_support::app_state().await;
    let (_b_dir, b) = crate::test_support::app_state().await;
    let alice = registration(&router, &alice_cookie, &a).await;
    let source_user = db::model::user::User::default();
    let source_identity = db::user_db::get_sync_identity(&a.db, source_user.id)
        .await
        .unwrap()
        .unwrap();
    db::sync_peer::bind_peer(
        &a.db,
        &source_user,
        &source_identity,
        &alice.server_identity,
    )
    .await
    .unwrap();
    db::sync_transport::activate_writer(&a.db, &source_user, &source_identity, &alice)
        .await
        .unwrap();
    let group = db::group_db::add_empty_group(&a.db, &source_user, "private", None)
        .await
        .unwrap();
    let root = db::causal_group_lifecycle::enroll_pristine_group(
        &a.db,
        &source_user,
        &source_identity,
        group.id,
    )
    .await
    .unwrap();
    let item = db::sync_transport::encode_genesis(&root).unwrap();
    let response = router
        .clone()
        .oneshot(publish(&alice, &item, &alice_cookie, &alice.writer_secret))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let password = db::random_hex_32();
    db::user_db::add_user(&hub.db, "bob", "bob-transport@example.test", &password)
        .await
        .unwrap();
    let credentials = serde_json::json!({"email":"bob-transport@example.test","password":password,"group_deletion_protocol":1});
    let login = router
        .clone()
        .oneshot(post("/api/v1/login/password", &credentials, None, None))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::NO_CONTENT);
    let bob_cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let bob = registration(&router, &bob_cookie, &b).await;
    let path = format!(
        "/api/v2/sync/items?protocol=1&connection_id={}&after=0&limit=64",
        bob.connection_id
    );
    let request = Request::get(path)
        .header(header::COOKIE, &bob_cookie)
        .header("x-sync-writer-secret", &bob.writer_secret)
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: serde_json::Value = serde_json::from_slice(
        &to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        page["items"].as_array().unwrap().as_slice(),
        [] as [serde_json::Value; 0]
    );
    let response = router
        .oneshot(publish(&bob, &item, &bob_cookie, &bob.writer_secret))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "another account cannot replay an owner's private event"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn real_rename_projects_and_log_failure_rolls_back_before_retry() {
    use db::causal_group_rename::NameState;
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let writer = registration(&router, &cookie, &source).await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    db::sync_peer::bind_peer(&source.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    db::sync_transport::activate_writer(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    let group = db::group_db::add_empty_group(&source.db, &user, "original", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    let genesis = db::sync_transport::encode_genesis(&root).unwrap();
    let response = router
        .clone()
        .oneshot(publish(&writer, &genesis, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    db::group_db::edit_group(&source.db, &user, group.id, "renamed", None, None)
        .await
        .unwrap();
    let state = db::causal_group_lifecycle::get_state(&source.db, &user, &identity, group.id)
        .await
        .unwrap();
    let dot = match state.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        other => panic!("expected ready rename: {other:?}"),
    };
    assert_eq!(dot.writer_id, writer.writer_id);
    let operation =
        db::causal_group_lifecycle::export_operation(&source.db, &user, &identity, &dot)
            .await
            .unwrap();
    let item = db::sync_transport::encode_operation(&operation).unwrap();
    let owner = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let before: (String,i64,i64,i64) = sqlx::query_as("SELECT group_name,(SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log),(SELECT counter FROM group_causal_replica WHERE singleton=1) FROM models_group WHERE group_user_id=? AND group_unique_global_id=?").bind(owner.id).bind(&root.entity().global_id).fetch_one(&*hub.db).await.unwrap();
    assert_eq!(before.0, "original");
    sqlx::query("CREATE TRIGGER reject_transport_log BEFORE INSERT ON sync_transport_log BEGIN SELECT RAISE(ABORT,'injected log failure'); END").execute(&*hub.db).await.unwrap();
    let failed = router
        .clone()
        .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_ne!(failed.status(), StatusCode::OK);
    let after: (String,i64,i64,i64) = sqlx::query_as("SELECT group_name,(SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log),(SELECT counter FROM group_causal_replica WHERE singleton=1) FROM models_group WHERE group_user_id=? AND group_unique_global_id=?").bind(owner.id).bind(&root.entity().global_id).fetch_one(&*hub.db).await.unwrap();
    assert_eq!(
        after, before,
        "failed delivery log must roll back projection and journal"
    );
    sqlx::query("DROP TRIGGER reject_transport_log")
        .execute(&*hub.db)
        .await
        .unwrap();
    let accepted = router
        .clone()
        .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    let name: String = sqlx::query_scalar(
        "SELECT group_name FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(owner.id)
    .bind(&root.entity().global_id)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(name, "renamed");
    let path = format!(
        "/api/v2/sync/items?protocol=1&connection_id={}&after=0&limit=64",
        writer.connection_id
    );
    let request = Request::get(path)
        .header(header::COOKIE, &cookie)
        .header("x-sync-writer-secret", &writer.writer_secret)
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: serde_json::Value = serde_json::from_slice(
        &to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["item"] == item),
        "the original rename must be relayed"
    );
}

#[tokio::test]
async fn writer_activation_rejects_registration_for_an_unpinned_server_account() {
    let (_hub_dir, _hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let registration = registration(&router, &cookie, &source).await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    db::sync_peer::bind_peer(&source.db, &user, &identity, &registration.server_identity)
        .await
        .unwrap();
    let before: (String, i64) =
        sqlx::query_as("SELECT replica_id,counter FROM group_causal_replica WHERE singleton=1")
            .fetch_one(&*source.db)
            .await
            .unwrap();
    let mut forged = registration.clone();
    forged.server_identity.account_id = db::random_hex_32();
    assert!(
        db::sync_transport::activate_writer(&source.db, &user, &identity, &forged)
            .await
            .is_err(),
        "a registration claiming an unpinned server account must not activate a writer"
    );
    let after: (String, i64) =
        sqlx::query_as("SELECT replica_id,counter FROM group_causal_replica WHERE singleton=1")
            .fetch_one(&*source.db)
            .await
            .unwrap();
    assert_eq!(after, before);
    db::sync_transport::activate_writer(&source.db, &user, &identity, &registration)
        .await
        .unwrap();
}

#[tokio::test]
async fn copied_client_identity_does_not_impersonate_another_writer_origin() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_a_dir, a) = crate::test_support::app_state().await;
    let (_b_dir, b) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let a_identity = db::user_db::get_sync_identity(&a.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let b_identity = db::user_db::get_sync_identity(&b.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let claim = serde_json::json!({"protocol":1,"client_identity":a_identity});
    let mut writers = Vec::new();
    for _ in 0..2 {
        let response = router
            .clone()
            .oneshot(post("/api/v2/sync/handshake", &claim, Some(&cookie), None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        writers.push(
            serde_json::from_slice::<db::sync_transport::WriterRegistration>(
                &to_bytes(response.into_body(), 8192).await.unwrap(),
            )
            .unwrap(),
        );
    }
    assert_ne!(writers[0].connection_id, writers[1].connection_id);
    let mut roots = Vec::new();
    for (client, identity, writer, name) in [
        (&a, &a_identity, &writers[0], "first"),
        (&b, &b_identity, &writers[1], "second"),
    ] {
        db::sync_peer::bind_peer(&client.db, &user, identity, &writer.server_identity)
            .await
            .unwrap();
        db::sync_transport::activate_writer(&client.db, &user, identity, writer)
            .await
            .unwrap();
        let group = db::group_db::add_empty_group(&client.db, &user, name, None)
            .await
            .unwrap();
        roots.push(
            db::causal_group_lifecycle::enroll_pristine_group(
                &client.db, &user, identity, group.id,
            )
            .await
            .unwrap(),
        );
    }
    let first = db::sync_transport::encode_genesis(&roots[0]).unwrap();
    let forged = router
        .clone()
        .oneshot(publish(
            &writers[1],
            &first,
            &cookie,
            &writers[1].writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(
        forged.status(),
        StatusCode::FORBIDDEN,
        "claiming a device identity does not grant its writer credential"
    );
    for (root, writer) in roots.iter().zip(&writers) {
        let item = db::sync_transport::encode_genesis(root).unwrap();
        let response = router
            .clone()
            .oneshot(publish(writer, &item, &cookie, &writer.writer_secret))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let owner = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let mut provenance = Vec::new();
    for root in &roots {
        let origin: (String,String) = sqlx::query_as("SELECT source_instance_id,source_account_id FROM causal_genesis_bindings WHERE user_id=? AND entity_kind='group' AND global_id=?").bind(owner.id).bind(&root.entity().global_id).fetch_one(&*hub.db).await.unwrap();
        provenance.push(origin);
    }
    assert_ne!(
        provenance[0], provenance[1],
        "copied client claims must not collapse two writer origins"
    );
    for (origin, writer) in provenance.iter().zip(&writers) {
        assert_eq!(origin.0, writer.connection_id);
        assert_eq!(origin.1, writer.server_identity.account_id);
    }
}

#[tokio::test]
async fn missing_genesis_is_requested_without_ack_or_writes_then_retry_succeeds() {
    use db::causal_group_rename::NameState;
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let writer = registration(&router, &cookie, &source).await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    db::sync_peer::bind_peer(&source.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    db::sync_transport::activate_writer(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    let group = db::group_db::add_empty_group(&source.db, &user, "original", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    db::group_db::edit_group(&source.db, &user, group.id, "renamed", None, None)
        .await
        .unwrap();
    let state = db::causal_group_lifecycle::get_state(&source.db, &user, &identity, group.id)
        .await
        .unwrap();
    let dot = match state.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        other => panic!("expected rename: {other:?}"),
    };
    let operation =
        db::causal_group_lifecycle::export_operation(&source.db, &user, &identity, &dot)
            .await
            .unwrap();
    let item = db::sync_transport::encode_operation(&operation).unwrap();
    let before: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log)").fetch_one(&*hub.db).await.unwrap();
    let response = router
        .clone()
        .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    let missing = serde_json::to_value(db::causal_envelope::Dependency {
        entity: root.entity().clone(),
        event: root.event().clone(),
    })
    .unwrap();
    assert!(
        body["missing"]
            .as_array()
            .is_some_and(|list| list.contains(&missing)),
        "response must identify the original missing genesis so the sender can retry"
    );
    let after: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log)").fetch_one(&*hub.db).await.unwrap();
    assert_eq!(after, before);
    let genesis = db::sync_transport::encode_genesis(&root).unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(publish(&writer, &genesis, &cookie, &writer.writer_secret))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        router
            .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn pre_registration_event_is_not_silently_reauthored_or_published() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = db::group_db::add_empty_group(&source.db, &user, "before registration", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    let original = root.event().clone();
    let item = db::sync_transport::encode_genesis(&root).unwrap();
    let writer = registration(&router, &cookie, &source).await;
    db::sync_peer::bind_peer(&source.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    db::sync_transport::activate_writer(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    assert_ne!(original.writer_id, writer.writer_id);
    let before: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log)",
    )
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    let response = router
        .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let after: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log)",
    )
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(after, before);
    let original_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM causal_events WHERE writer_id=? AND counter=?")
            .bind(&original.writer_id)
            .bind(original.counter.parse::<i64>().unwrap())
            .fetch_one(&*source.db)
            .await
            .unwrap();
    assert_eq!(original_count, 1);
}

#[tokio::test]
async fn committed_items_and_sessions_survive_hub_database_reopen() {
    let (hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let writer = registration(&router, &cookie, &source).await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    db::sync_peer::bind_peer(&source.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    db::sync_transport::activate_writer(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    let group = db::group_db::add_empty_group(&source.db, &user, "persisted", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    let item = db::sync_transport::encode_genesis(&root).unwrap();
    let response = router
        .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let reopened = db::db_context::setup_db(
        &hub_dir.path().join("db.sqlite"),
        &hub_dir.path().join("backups"),
    )
    .await;
    let mut state = hub.clone();
    state.db = std::sync::Arc::new(reopened);
    let router = crate::test_support::with_session(
        crate::controller::auth_controller::login_router().merge(super::router()),
        state,
    )
    .await;
    let path = format!(
        "/api/v2/sync/items?protocol=1&connection_id={}&after=0&limit=64",
        writer.connection_id
    );
    let request = Request::get(path)
        .header(header::COOKIE, &cookie)
        .header("x-sync-writer-secret", &writer.writer_secret)
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: serde_json::Value = serde_json::from_slice(
        &to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(page["items"][0]["item"], item);
}

#[tokio::test]
async fn writer_activation_rejects_noncanonical_registration_without_writes() {
    for field in ["writer_id", "connection_id", "writer_secret"] {
        let (_hub_dir, _hub, router, cookie) = app().await;
        let (_source_dir, source) = crate::test_support::app_state().await;
        let registration = registration(&router, &cookie, &source).await;
        let user = db::model::user::User::default();
        let identity = db::user_db::get_sync_identity(&source.db, user.id)
            .await
            .unwrap()
            .unwrap();
        db::sync_peer::bind_peer(&source.db, &user, &identity, &registration.server_identity)
            .await
            .unwrap();
        let before: (String,i64,i64) = sqlx::query_as("SELECT replica_id,counter,(SELECT COUNT(*) FROM sync_transport_local_writers) FROM group_causal_replica WHERE singleton=1").fetch_one(&*source.db).await.unwrap();
        let mut malformed = registration.clone();
        match field {
            "writer_id" => malformed.writer_id = "z".repeat(32),
            "connection_id" => malformed.connection_id = "z".repeat(32),
            "writer_secret" => malformed.writer_secret = "z".repeat(64),
            _ => unreachable!(),
        }
        assert!(
            db::sync_transport::activate_writer(&source.db, &user, &identity, &malformed)
                .await
                .is_err(),
            "noncanonical {field} must not activate"
        );
        let after: (String,i64,i64) = sqlx::query_as("SELECT replica_id,counter,(SELECT COUNT(*) FROM sync_transport_local_writers) FROM group_causal_replica WHERE singleton=1").fetch_one(&*source.db).await.unwrap();
        assert_eq!(after, before);
        db::sync_transport::activate_writer(&source.db, &user, &identity, &registration)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn account_deletion_cascades_transport_state_and_reused_id_rejects_old_writer() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let writer = registration(&router, &cookie, &source).await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    db::sync_peer::bind_peer(&source.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    db::sync_transport::activate_writer(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    let group = db::group_db::add_empty_group(&source.db, &user, "published", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    let item = db::sync_transport::encode_genesis(&root).unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let owner = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let before: (i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM sync_transport_writers WHERE user_id=?),(SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?)").bind(owner.id).bind(owner.id).fetch_one(&*hub.db).await.unwrap();
    assert_eq!(before, (1, 1));
    sqlx::query("DELETE FROM users WHERE user_id=?")
        .bind(owner.id)
        .execute(&*hub.db)
        .await
        .unwrap();
    let after: (i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM sync_transport_writers WHERE user_id=?),(SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?)").bind(owner.id).bind(owner.id).fetch_one(&*hub.db).await.unwrap();
    assert_eq!(after, (0, 0));
    let replacement_password = db::random_hex_32();
    let password_hash = db::model::user::hash_password(&replacement_password);
    sqlx::query("INSERT INTO users (user_id,user_name,user_email,user_password_hash,user_created_at) VALUES (?,'replacement','replacement-transport@example.test',?,'2020-01-01T00:00:00Z')").bind(owner.id).bind(password_hash).execute(&*hub.db).await.unwrap();
    let credentials = serde_json::json!({"email":"replacement-transport@example.test","password":replacement_password,"group_deletion_protocol":1});
    let login = router
        .clone()
        .oneshot(post("/api/v1/login/password", &credentials, None, None))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::NO_CONTENT);
    let replacement_cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let response = router
        .oneshot(publish(
            &writer,
            &item,
            &replacement_cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[path = "sync_offline_transport_tests.rs"]
mod offline;
