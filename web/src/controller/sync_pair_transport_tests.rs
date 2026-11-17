//! The user-initiated account link must bind a real authenticated TLS peer atomically.
use super::{
    pull::{TEST_CA, TlsHub, publish_observed_source},
    *,
};

type LinkState = (Option<String>, Option<String>, i64, i64, i64);

async fn link_state(state: &service::AppState, user: &db::model::user::User) -> LinkState {
    sqlx::query_as("SELECT user_sync_url,user_sync_token,(SELECT COUNT(*) FROM sync_peer_bindings WHERE user_id=?),(SELECT COUNT(*) FROM sync_transport_local_authorities WHERE user_id=?),(SELECT COUNT(*) FROM causal_account_writers WHERE user_id=?) FROM users WHERE user_id=?")
        .bind(user.id).bind(user.id).bind(user.id).bind(user.id)
        .fetch_one(&*state.db).await.unwrap()
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Authentication, first pull and same-link retry form one end-to-end scenario.
async fn user_initiated_pair_establishes_one_authenticated_link_and_pulls() {
    use service::sync_pull_service::{AuthenticatedHub, HubTrust, PullOutcome};
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, _source, global, _, _) = publish_observed_source(&router, &cookie).await;
    let (_local_dir, local) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&local.db, user.id)
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
    assert_eq!(link_state(&local, &user).await, (None, None, 0, 0, 0));
    let client = AuthenticatedHub::pair(
        &local,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .expect("the existing link action should authenticate and pair");
    assert_eq!(
        link_state(&local, &user).await,
        (Some(server.origin()), Some(token.clone()), 1, 1, 1)
    );
    let stored:(String,String,String)=sqlx::query_as("SELECT server_instance_id,server_account_id,connection_id FROM sync_transport_local_authorities WHERE user_id=?").bind(user.id).fetch_one(&*local.db).await.unwrap();
    let remote = db::user_db::get_sync_identity(&hub.db, hub_user.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (&stored.0, &stored.1),
        (&remote.instance_id, &remote.account_id)
    );
    assert!(!format!("{client:?}").contains(&token));
    assert_eq!(
        client.pull_next(&local, &user, &identity).await.unwrap(),
        PullOutcome::Applied {
            sequence: "1".into()
        }
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(user.id)
    .bind(global)
    .fetch_one(&*local.db)
    .await
    .unwrap();
    assert_eq!(count, 1);
    let registrations_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_transport_writers WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    let writer_before: (String, i64) =
        sqlx::query_as("SELECT writer_id,counter FROM causal_account_writers WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*local.db)
            .await
            .unwrap();
    let again = AuthenticatedHub::pair(
        &local,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let stored_after:(String,String,String)=sqlx::query_as("SELECT server_instance_id,server_account_id,connection_id FROM sync_transport_local_authorities WHERE user_id=?").bind(user.id).fetch_one(&*local.db).await.unwrap();
    assert_eq!(
        stored, stored_after,
        "an exact retry must not rotate the writer"
    );
    let registrations_after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_transport_writers WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    let writer_after: (String, i64) =
        sqlx::query_as("SELECT writer_id,counter FROM causal_account_writers WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*local.db)
            .await
            .unwrap();
    assert_eq!(
        registrations_before, registrations_after,
        "an exact retry cannot create another remote grant"
    );
    assert_eq!(
        writer_before, writer_after,
        "an exact retry cannot rotate or advance the local writer"
    );
    assert!(matches!(
        again.pull_next(&local, &user, &identity).await.unwrap(),
        PullOutcome::Idle { .. }
    ));
}

#[tokio::test]
async fn failed_authentication_never_installs_a_local_link() {
    use service::sync_pull_service::{AuthenticatedHub, HubTrust};
    let (_hub_dir, hub, router, _) = app().await;
    let (_local_dir, local) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&local.db, user.id)
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
    let before = link_state(&local, &user).await;
    let trust = || HubTrust::AdditionalCaPem(TEST_CA.to_vec());
    assert!(
        AuthenticatedHub::pair(
            &local,
            &user,
            &identity,
            &server.origin(),
            &db::random_hex_32(),
            trust()
        )
        .await
        .is_err()
    );
    assert_eq!(link_state(&local, &user).await, before);
    assert!(
        AuthenticatedHub::pair(
            &local,
            &user,
            &identity,
            &server.origin(),
            &token,
            HubTrust::System
        )
        .await
        .is_err()
    );
    assert_eq!(link_state(&local, &user).await, before);
    let wrong_hostname = server.origin().replace("localhost", "127.0.0.1");
    assert!(
        AuthenticatedHub::pair(&local, &user, &identity, &wrong_hostname, &token, trust())
            .await
            .is_err()
    );
    assert_eq!(link_state(&local, &user).await, before);
    let insecure = server.origin().replace("https://", "http://");
    assert!(
        AuthenticatedHub::pair(&local, &user, &identity, &insecure, &token, trust())
            .await
            .is_err()
    );
    assert_eq!(link_state(&local, &user).await, before);
    assert!(
        AuthenticatedHub::pair(&local, &user, &identity, &server.origin(), &token, trust())
            .await
            .is_ok()
    );
    let linked = link_state(&local, &user).await;
    assert!(
        AuthenticatedHub::pair(
            &local,
            &user,
            &identity,
            &server.origin(),
            &db::random_hex_32(),
            trust()
        )
        .await
        .is_err()
    );
    assert_eq!(
        link_state(&local, &user).await,
        linked,
        "failed relink must retain the existing credentials"
    );
}

#[tokio::test]
async fn pairing_retry_rejects_revoked_remote_grant_without_rotating_local_writer() {
    use service::sync_pull_service::{AuthenticatedHub, HubTrust};
    let (_hub_dir, hub, router, _) = app().await;
    let (_local_dir, local) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&local.db, user.id)
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
    AuthenticatedHub::pair(
        &local,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let before = link_state(&local, &user).await;
    let writer: (String, i64) =
        sqlx::query_as("SELECT writer_id,counter FROM causal_account_writers WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*local.db)
            .await
            .unwrap();
    let connection: String = sqlx::query_scalar(
        "SELECT connection_id FROM sync_transport_local_authorities WHERE user_id=?",
    )
    .bind(user.id)
    .fetch_one(&*local.db)
    .await
    .unwrap();
    let registrations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_transport_writers WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    sqlx::query(
        "UPDATE sync_transport_writers SET secret_hash=? WHERE user_id=? AND connection_id=?",
    )
    .bind("f".repeat(64))
    .bind(hub_user.id)
    .bind(connection)
    .execute(&*hub.db)
    .await
    .unwrap();
    assert!(
        AuthenticatedHub::pair(
            &local,
            &user,
            &identity,
            &server.origin(),
            &token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec())
        )
        .await
        .is_err(),
        "revoked writer grant cannot be reported as usable"
    );
    assert_eq!(link_state(&local, &user).await, before);
    let after: (String, i64) =
        sqlx::query_as("SELECT writer_id,counter FROM causal_account_writers WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*local.db)
            .await
            .unwrap();
    assert_eq!(writer, after);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transport_writers WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap(),
        registrations
    );
}

#[tokio::test]
async fn forged_handshake_or_redirect_cannot_commit_pairing() {
    use axum::{
        middleware::{self, Next},
        response::IntoResponse,
    };
    use service::sync_pull_service::{AuthenticatedHub, HubTrust};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (_hub_dir, hub, router, _) = app().await;
    let (_local_dir, local) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&local.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let mode = Arc::new(AtomicUsize::new(1));
    let state = mode.clone();
    let reached = Arc::new(AtomicUsize::new(0));
    let seen = reached.clone();
    let router = router.layer(middleware::from_fn(
        move |request: Request<Body>, next: Next| {
            let (state, seen) = (state.clone(), seen.clone());
            async move {
                let handshake = request.uri().path() == "/api/v2/sync/handshake";
                if !handshake {
                    return next.run(request).await;
                }
                seen.fetch_add(1, Ordering::SeqCst);
                if state.load(Ordering::SeqCst) == 2 {
                    return (
                        StatusCode::TEMPORARY_REDIRECT,
                        [(header::LOCATION, "https://attacker.invalid/")],
                    )
                        .into_response();
                }
                let response = next.run(request).await;
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
                let mut body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                body["server_identity"]["account_id"] = "f".repeat(32).into();
                axum::Json(body).into_response()
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
    let before = link_state(&local, &user).await;
    for selected in [1, 2] {
        mode.store(selected, Ordering::SeqCst);
        assert!(
            AuthenticatedHub::pair(
                &local,
                &user,
                &identity,
                &server.origin(),
                &token,
                HubTrust::AdditionalCaPem(TEST_CA.to_vec())
            )
            .await
            .is_err()
        );
        assert_eq!(link_state(&local, &user).await, before);
    }
    assert_eq!(
        reached.load(Ordering::SeqCst),
        2,
        "both attacks must reach the registration response"
    );
}

#[tokio::test]
async fn failed_authority_commit_rolls_back_url_token_and_binding() {
    use service::sync_pull_service::{AuthenticatedHub, HubTrust};
    let (_hub_dir, hub, router, _) = app().await;
    let (_local_dir, local) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&local.db, user.id)
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
    let before = link_state(&local, &user).await;
    sqlx::query("CREATE TRIGGER fail_pair BEFORE INSERT ON sync_transport_local_authorities BEGIN SELECT RAISE(ABORT,'injected authority failure'); END").execute(&*local.db).await.unwrap();
    assert!(
        AuthenticatedHub::pair(
            &local,
            &user,
            &identity,
            &server.origin(),
            &token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec())
        )
        .await
        .is_err()
    );
    assert_eq!(link_state(&local, &user).await, before);
    sqlx::query("DROP TRIGGER fail_pair")
        .execute(&*local.db)
        .await
        .unwrap();
    assert!(
        AuthenticatedHub::pair(
            &local,
            &user,
            &identity,
            &server.origin(),
            &token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec())
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One handshake barrier covers cancellation and both local identity races.
async fn cancelled_or_stale_pairing_cannot_install_credentials() {
    use axum::middleware::{self, Next};
    use service::sync_pull_service::{AuthenticatedHub, HubTrust};
    use std::sync::Arc;
    use tokio::sync::Notify;
    for mode in [0, 1, 2] {
        let (_hub_dir, hub, router, _) = app().await;
        let (_local_dir, local) = crate::test_support::app_state().await;
        let user = db::model::user::User::default();
        let identity = db::user_db::get_sync_identity(&local.db, user.id)
            .await
            .unwrap()
            .unwrap();
        let reached = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (arrived, continue_request) = (reached.clone(), release.clone());
        let router = router.layer(middleware::from_fn(
            move |request: Request<Body>, next: Next| {
                let (arrived, continue_request) = (arrived.clone(), continue_request.clone());
                async move {
                    let handshake = request.uri().path() == "/api/v2/sync/handshake";
                    let response = next.run(request).await;
                    if handshake {
                        arrived.notify_one();
                        continue_request.notified().await;
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
        let before = link_state(&local, &user).await;
        let origin = server.origin();
        let mut pair = Box::pin(AuthenticatedHub::pair(
            &local,
            &user,
            &identity,
            &origin,
            &token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
        ));
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::select! {
                ()=reached.notified()=>{},
                result=&mut pair=>panic!("pair finished before registration barrier: {result:?}"),
            }
        })
        .await
        .expect("handshake must reach the barrier");
        if mode == 0 {
            drop(pair);
            release.notify_one();
        } else {
            if mode == 1 {
                db::user_db::set_user_sync_token(
                    &local.db,
                    user.id,
                    &db::random_hex_32(),
                    &origin,
                    false,
                )
                .await
                .unwrap();
                db::user_db::clear_user_sync(&local.db, user.id)
                    .await
                    .unwrap();
            } else {
                sqlx::query("DELETE FROM sync_accounts WHERE user_id=?")
                    .bind(user.id)
                    .execute(&*local.db)
                    .await
                    .unwrap();
                sqlx::query("INSERT INTO sync_accounts(user_id,account_id) VALUES(?,?)")
                    .bind(user.id)
                    .bind(db::random_hex_32())
                    .execute(&*local.db)
                    .await
                    .unwrap();
            }
            assert_eq!(
                link_state(&local, &user).await,
                before,
                "the link values have returned to their original state"
            );
            release.notify_one();
            assert!(
                pair.await.is_err(),
                "link change or account replacement must invalidate pending pairing"
            );
        }
        assert_eq!(
            link_state(&local, &user).await,
            before,
            "cancelled or stale pairing must leave no local credentials"
        );
    }
}

#[tokio::test]
async fn competing_pair_attempts_cannot_install_two_local_authorities() {
    use axum::middleware::{self, Next};
    use service::sync_pull_service::{AuthenticatedHub, HubTrust};
    use std::sync::Arc;
    let (_hub_dir, hub, router, _) = app().await;
    let (_local_dir, local) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&local.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let router = router.layer(middleware::from_fn(
        move |request: Request<Body>, next: Next| {
            let barrier = barrier.clone();
            async move {
                let handshake = request.uri().path() == "/api/v2/sync/handshake";
                let response = next.run(request).await;
                if handshake {
                    barrier.wait().await;
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
    let origin = server.origin();
    let pair = || {
        AuthenticatedHub::pair(
            &local,
            &user,
            &identity,
            &origin,
            &token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
        )
    };
    let (first, second) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(pair(), pair())
    })
    .await
    .expect("both handshakes must reach the barrier");
    assert_eq!(
        usize::from(first.is_ok()) + usize::from(second.is_ok()),
        1,
        "only the attempt that commits the captured link generation may succeed"
    );
    assert_eq!(
        link_state(&local, &user).await,
        (Some(origin), Some(token), 1, 1, 1)
    );
}
