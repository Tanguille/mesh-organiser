//! Independent paired devices publish and consume their original operations over HTTPS.
use std::sync::{Arc, atomic::Ordering};

use axum::{
    middleware::{self, Next},
    response::IntoResponse,
};
use tokio::sync::Notify;

use db::causal_group_rename::NameState;
use service::sync_pull_service::{AuthenticatedHub, HubTrust, PublicationOutcome, PullOutcome};

use super::{
    pull::{TEST_CA, TlsHub},
    *,
};

#[tokio::test]
#[allow(clippy::too_many_lines)] // Observation, rename, deletion and restoration must preserve one ordered author history.
async fn paired_author_publishes_original_history_for_an_independent_receiver() {
    let (_hub_dir, hub, router, _) = app().await;
    let (author_dir, author) = crate::test_support::app_state().await;
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let (_other_dir, other) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let ai = db::user_db::get_sync_identity(&author.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let ri = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let oi = db::user_db::get_sync_identity(&other.db, user.id)
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
    let other_token = db::random_hex_32();
    let other_hub_user =
        db::user_db::add_user(&hub.db, "bob", "bob@example.com", &db::random_hex_32())
            .await
            .unwrap();
    db::user_db::set_user_sync_token(
        &hub.db,
        other_hub_user,
        &other_token,
        &server.origin(),
        false,
    )
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
    let other_client = AuthenticatedHub::pair(
        &other,
        &user,
        &oi,
        &server.origin(),
        &other_token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let global = db::random_hex_32();
    let group=sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('original','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(&global).execute(&*author.db).await.unwrap().last_insert_rowid();
    let file = author_dir.path().join("model.stl");
    std::fs::write(&file, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &author.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        Some(file.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let model = db::model_db::add_model(&author.db, &user, "part", blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&author.db, &user, Some(group), vec![model], None)
        .await
        .unwrap();
    let prepared =
        service::sync_observation_service::prepare_observed_group(&author, &user, &ai, group)
            .await
            .unwrap();
    assert!(
        author_client
            .publish_observation(&receiver, &user, &ri, &prepared)
            .await
            .is_err(),
        "an author client cannot publish using another local account"
    );
    assert!(
        receiver_client
            .publish_observation(&receiver, &user, &ri, &prepared)
            .await
            .is_err(),
        "a receiver cannot reauthor another device's prepared item"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap(),
        0
    );
    let staged = prepared.blobs()[0].path();
    std::fs::write(staged, b"abd").unwrap();
    assert!(
        author_client
            .publish_observation(&author, &user, &ai, &prepared)
            .await
            .is_err(),
        "changed staged bytes must not be acknowledged"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap(),
        0
    );
    std::fs::write(staged, b"abc").unwrap();
    assert_eq!(
        author_client
            .publish_observation(&author, &user, &ai, &prepared)
            .await
            .unwrap(),
        PublicationOutcome::Published {
            sequence: "1".into()
        }
    );
    assert_eq!(
        author_client
            .publish_observation(&author, &user, &ai, &prepared)
            .await
            .unwrap(),
        PublicationOutcome::AlreadyPresent {
            sequence: "1".into()
        }
    );
    drop(prepared);
    assert_eq!(
        receiver_client
            .pull_next(&receiver, &user, &ri)
            .await
            .unwrap(),
        PullOutcome::Applied {
            sequence: "1".into()
        }
    );
    assert!(
        matches!(
            other_client.pull_next(&other, &user, &oi).await.unwrap(),
            PullOutcome::Idle { .. }
        ),
        "another authenticated account must not see the author's item"
    );
    let receiver_group: i64 = sqlx::query_scalar(
        "SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    let receiver_model: i64 = sqlx::query_scalar(
        "SELECT model_id FROM models WHERE model_user_id=? AND model_group_id=?",
    )
    .bind(user.id)
    .bind(receiver_group)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    db::group_db::edit_group(&author.db, &user, group, "renamed", None, None)
        .await
        .unwrap();
    let NameState::Ready { token, .. } =
        db::causal_group_lifecycle::get_state(&author.db, &user, &ai, group)
            .await
            .unwrap()
            .name
    else {
        panic!("expected rename")
    };
    let rename = token.frontier[0].clone();
    assert_eq!(
        author_client
            .publish_operation(&author, &user, &ai, &rename)
            .await
            .unwrap(),
        PublicationOutcome::Published {
            sequence: "2".into()
        }
    );
    assert_eq!(
        author_client
            .publish_operation(&author, &user, &ai, &rename)
            .await
            .unwrap(),
        PublicationOutcome::AlreadyPresent {
            sequence: "2".into()
        }
    );
    assert_eq!(
        receiver_client
            .pull_next(&receiver, &user, &ri)
            .await
            .unwrap(),
        PullOutcome::Applied {
            sequence: "2".into()
        }
    );
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=?")
        .bind(receiver_group)
        .fetch_one(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(name, "renamed");
    db::group_db::delete_group(&author.db, &user, group)
        .await
        .unwrap();
    let db::causal_group_lifecycle::GroupLifecycle::Deleted { deletions } =
        db::causal_group_lifecycle::get_state(&author.db, &user, &ai, group)
            .await
            .unwrap()
            .lifecycle
    else {
        panic!("expected deletion")
    };
    assert_eq!(
        author_client
            .publish_operation(&author, &user, &ai, &deletions[0])
            .await
            .unwrap(),
        PublicationOutcome::Published {
            sequence: "3".into()
        }
    );
    assert_eq!(
        receiver_client
            .pull_next(&receiver, &user, &ri)
            .await
            .unwrap(),
        PullOutcome::Applied {
            sequence: "3".into()
        }
    );
    let membership: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(receiver_model)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    assert_eq!(membership, None, "deletion ungroups its former member");
    let deleted = db::group_db::get_trashed_groups(&author.db, &user, 30)
        .await
        .unwrap()
        .into_iter()
        .find(|entry| entry.id == group)
        .unwrap();
    let request = db::group_db::GroupRestoreRequest {
        id: deleted.id,
        group_global_id: deleted.group_global_id,
        deletion_operation_id: deleted.deletion_operation_id.unwrap(),
    };
    db::group_db::restore_groups(&author.db, &user, &[request], 30)
        .await
        .unwrap();
    let NameState::Ready { token, .. } =
        db::causal_group_lifecycle::get_state(&author.db, &user, &ai, group)
            .await
            .unwrap()
            .name
    else {
        panic!("expected restoration")
    };
    assert_eq!(
        author_client
            .publish_operation(&author, &user, &ai, &token.frontier[0])
            .await
            .unwrap(),
        PublicationOutcome::Published {
            sequence: "4".into()
        }
    );
    assert_eq!(
        receiver_client
            .pull_next(&receiver, &user, &ri)
            .await
            .unwrap(),
        PullOutcome::Applied {
            sequence: "4".into()
        }
    );
    let membership: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(receiver_model)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    assert_eq!(
        membership,
        Some(receiver_group),
        "restoration reattaches an ungrouped former member"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Missing dependencies, hub rollback and acknowledgement verification share one original event.
async fn publication_waits_for_dependencies_and_verifies_the_committed_acknowledgement() {
    let (_hub_dir, hub, router, _) = app().await;
    let (_author_dir, author) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let ai = db::user_db::get_sync_identity(&author.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let corrupt = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let state = corrupt.clone();
    let router = router.layer(middleware::from_fn(
        move |request: Request<Body>, next: Next| {
            let state = state.clone();
            async move {
                let page = request.uri().path() == "/api/v2/sync/items"
                    && request.method() == axum::http::Method::GET;
                let response = next.run(request).await;
                let mode = state.load(Ordering::SeqCst);
                if !page || mode == 0 {
                    return response;
                }
                let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
                    .await
                    .unwrap();
                let mut body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                match mode {
                    1 => body["items"][0]["digest"] = "0".repeat(64).into(),
                    2 => body["items"][0]["issuer"]["connection_id"] = "f".repeat(32).into(),
                    3 => body["items"][0]["issuer"]["authority_writer_id"] = "f".repeat(32).into(),
                    4 => body["items"][0]["issuer"]["writer_id"] = "f".repeat(32).into(),
                    5 => body["items"][0]["item"]["unexpected"] = true.into(),
                    6 => body["server_identity"]["account_id"] = "f".repeat(32).into(),
                    _ => unreachable!(),
                }
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
    let client = AuthenticatedHub::pair(
        &author,
        &user,
        &ai,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let group = db::group_db::add_empty_group(&author.db, &user, "original", None)
        .await
        .unwrap();
    let root = db::causal_group_lifecycle::enroll_pristine_group(&author.db, &user, &ai, group.id)
        .await
        .unwrap();
    db::group_db::edit_group(&author.db, &user, group.id, "renamed", None, None)
        .await
        .unwrap();
    let NameState::Ready { token, .. } =
        db::causal_group_lifecycle::get_state(&author.db, &user, &ai, group.id)
            .await
            .unwrap()
            .name
    else {
        panic!("expected rename")
    };
    let dot = token.frontier[0].clone();
    let result = client
        .publish_operation(&author, &user, &ai, &dot)
        .await
        .unwrap();
    assert!(
        matches!(result,PublicationOutcome::NeedsDependencies {ref missing} if !missing.is_empty())
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap(),
        0
    );
    sqlx::query("CREATE TRIGGER fail_publish_log BEFORE INSERT ON sync_transport_log BEGIN SELECT RAISE(ABORT,'injected publication failure'); END").execute(&*hub.db).await.unwrap();
    assert!(
        client
            .publish_genesis(&author, &user, &ai, &root)
            .await
            .is_err()
    );
    let before:(i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?),(SELECT COUNT(*) FROM models_group WHERE group_user_id=?)").bind(hub_user.id).bind(hub_user.id).fetch_one(&*hub.db).await.unwrap();
    assert_eq!(
        before,
        (0, 0),
        "hub log failure must roll back the genesis projection"
    );
    sqlx::query("DROP TRIGGER fail_publish_log")
        .execute(&*hub.db)
        .await
        .unwrap();
    for mode in 1..=6 {
        corrupt.store(mode, Ordering::SeqCst);
        assert!(
            client
                .publish_genesis(&author, &user, &ai, &root)
                .await
                .is_err(),
            "modified read-back mode {mode} cannot acknowledge publication"
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap(),
        1,
        "an unacknowledged request may still have committed remotely"
    );
    corrupt.store(0, Ordering::SeqCst);
    assert_eq!(
        client
            .publish_genesis(&author, &user, &ai, &root)
            .await
            .unwrap(),
        PublicationOutcome::AlreadyPresent {
            sequence: "1".into()
        }
    );
    assert_eq!(
        client
            .publish_operation(&author, &user, &ai, &dot)
            .await
            .unwrap(),
        PublicationOutcome::Published {
            sequence: "2".into()
        }
    );
}

#[tokio::test]
async fn link_replacement_during_publication_cannot_return_a_false_acknowledgement() {
    let (_hub_dir, hub, router, _) = app().await;
    let (_author_dir, author) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&author.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let reached = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let (arrived, proceed) = (reached.clone(), release.clone());
    let router = router.layer(middleware::from_fn(
        move |request: Request<Body>, next: Next| {
            let (arrived, proceed) = (arrived.clone(), proceed.clone());
            async move {
                let publish = request.uri().path() == "/api/v2/sync/items"
                    && request.method() == axum::http::Method::POST;
                let response = next.run(request).await;
                if publish {
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
        &author,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let group = db::group_db::add_empty_group(&author.db, &user, "folder", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&author.db, &user, &identity, group.id)
            .await
            .unwrap();
    let publish = client.publish_genesis(&author, &user, &identity, &root);
    let unlink = async {
        reached.notified().await;
        db::user_db::set_user_sync_token(
            &author.db,
            user.id,
            &db::random_hex_32(),
            &server.origin(),
            false,
        )
        .await
        .unwrap();
        db::user_db::set_user_sync_token(&author.db, user.id, &token, &server.origin(), false)
            .await
            .unwrap();
        release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(publish, unlink)
    })
    .await
    .expect("publication must reach the commit barrier");
    assert!(
        result.is_err(),
        "replaced link must not report a successful publication"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap(),
        1,
        "a lost acknowledgment does not imply remote rollback"
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM sync_pull_receipts WHERE stream_id IN (SELECT stream_id FROM sync_pull_streams WHERE user_id=?)").bind(user.id).fetch_one(&*author.db).await.unwrap(),0);
}
