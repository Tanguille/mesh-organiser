//! Real combined operations use complete manifests and preserve author identity.
use super::*;

fn canonical_context(value: &serde_json::Value) -> String {
    #[derive(serde::Serialize)]
    struct Context {
        version: u32,
        frontier: Vec<db::causal_envelope::Dependency>,
        events: Vec<db::causal_envelope::Dot>,
    }
    let context = Context {
        version: u32::try_from(value["version"].as_u64().unwrap()).unwrap(),
        frontier: serde_json::from_value(value["frontier"].clone()).unwrap(),
        events: serde_json::from_value(value["events"].clone()).unwrap(),
    };
    serde_json::to_string(&context).unwrap()
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Exercise a missing cross-entity cut followed by its real delivery.
async fn deletion_waits_for_missing_model_frontier_then_retries() {
    use db::causal_model_move::MoveState;
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
    let group = db::group_db::add_empty_group(&source.db, &user, "destination", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    let response = router
        .clone()
        .oneshot(publish(
            &writer,
            &db::sync_transport::encode_genesis(&root).unwrap(),
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let hub_identity = db::user_db::get_sync_identity(&hub.db, hub_user.id)
        .await
        .unwrap()
        .unwrap();
    let peer = db::sync_peer::bind_peer(&hub.db, &hub_user, &hub_identity, &identity)
        .await
        .unwrap();
    let hash = db::random_hex_32();
    let source_blob = db::blob_db::add_blob(&source.db, &hash, "stl", 1, None)
        .await
        .unwrap();
    let hub_blob = db::blob_db::add_blob(&hub.db, &hash, "stl", 1, None)
        .await
        .unwrap();
    let model = db::model_db::add_model(&source.db, &user, "part", source_blob, None, None)
        .await
        .unwrap();
    let model_root =
        db::causal_group_lifecycle::enroll_pristine_model(&source.db, &user, &identity, model)
            .await
            .unwrap();
    let target_model = db::causal_genesis::install_pristine_genesis(
        &hub.db,
        &hub_user,
        &hub_identity,
        &peer,
        &model_root,
        Some(hub_blob),
    )
    .await
    .unwrap()
    .local_id;
    db::group_db::set_group_id_on_models(&source.db, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    let MoveState::Ready { token, .. } =
        db::causal_model_move::get_state(&source.db, &user, &identity, model)
            .await
            .unwrap()
    else {
        panic!("expected initial move")
    };
    let into = db::causal_group_lifecycle::export_operation(
        &source.db,
        &user,
        &identity,
        &token.frontier[0],
    )
    .await
    .unwrap();
    let response = router
        .clone()
        .oneshot(publish(
            &writer,
            &db::sync_transport::encode_operation(&into).unwrap(),
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    db::group_db::set_group_id_on_models(&source.db, &user, None, vec![model], None)
        .await
        .unwrap();
    let MoveState::Ready { token, .. } =
        db::causal_model_move::get_state(&source.db, &user, &identity, model)
            .await
            .unwrap()
    else {
        panic!("expected move away")
    };
    let moved = db::causal_group_lifecycle::export_operation(
        &source.db,
        &user,
        &identity,
        &token.frontier[0],
    )
    .await
    .unwrap();
    let moved_item = db::sync_transport::encode_operation(&moved).unwrap();
    db::group_db::delete_group(&source.db, &user, group.id)
        .await
        .unwrap();
    let db::causal_group_lifecycle::GroupLifecycle::Deleted { deletions } =
        db::causal_group_lifecycle::get_state(&source.db, &user, &identity, group.id)
            .await
            .unwrap()
            .lifecycle
    else {
        panic!("expected deletion")
    };
    let deletion =
        db::causal_group_lifecycle::export_operation(&source.db, &user, &identity, &deletions[0])
            .await
            .unwrap();
    let delete_item = db::sync_transport::encode_operation(&deletion).unwrap();
    let before: (i64,i64,Option<i64>) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log),model_group_id FROM models WHERE model_id=? AND model_user_id=?").bind(target_model).bind(hub_user.id).fetch_one(&*hub.db).await.unwrap();
    let missing = router
        .clone()
        .oneshot(publish(
            &writer,
            &delete_item,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::CONFLICT);
    let missing: serde_json::Value =
        serde_json::from_slice(&to_bytes(missing.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(missing["status"], "needs_dependencies");
    assert!(
        missing["missing"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |entry| entry["event"]["writer_id"] == token.frontier[0].writer_id
                    && entry["event"]["counter"] == token.frontier[0].counter
            )
    );
    let after: (i64,i64,Option<i64>) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log),model_group_id FROM models WHERE model_id=? AND model_user_id=?").bind(target_model).bind(hub_user.id).fetch_one(&*hub.db).await.unwrap();
    assert_eq!(
        after, before,
        "missing context must not admit or acknowledge deletion"
    );
    let response = router
        .clone()
        .oneshot(publish(
            &writer,
            &moved_item,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = router
        .oneshot(publish(
            &writer,
            &delete_item,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the same deletion succeeds after the missing move arrives"
    );
    let destination: Option<i64> = sqlx::query_scalar(
        "SELECT model_group_id FROM models WHERE model_id=? AND model_user_id=?",
    )
    .bind(target_model)
    .bind(hub_user.id)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(destination, None);
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Verify a late original rename after a real retention purge.
async fn concurrent_rename_rehydrates_expired_group_through_transport() {
    use db::causal_group_lifecycle::GroupLifecycle;
    use db::causal_group_rename::NameState;
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_a_dir, a) = crate::test_support::app_state().await;
    let (_b_dir, b) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let ai = db::user_db::get_sync_identity(&a.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let bi = db::user_db::get_sync_identity(&b.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let aw = registration(&router, &cookie, &a).await;
    let bw = registration(&router, &cookie, &b).await;
    for (device, identity, writer) in [(&a, &ai, &aw), (&b, &bi, &bw)] {
        db::sync_peer::bind_peer(&device.db, &user, identity, &writer.server_identity)
            .await
            .unwrap();
        db::sync_transport::activate_writer(&device.db, &user, identity, writer)
            .await
            .unwrap();
    }
    let group = db::group_db::add_empty_group(&a.db, &user, "original", None)
        .await
        .unwrap();
    let root = db::causal_group_lifecycle::enroll_pristine_group(&a.db, &user, &ai, group.id)
        .await
        .unwrap();
    let response = router
        .clone()
        .oneshot(publish(
            &aw,
            &db::sync_transport::encode_genesis(&root).unwrap(),
            &cookie,
            &aw.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ba = db::sync_peer::bind_peer(&b.db, &user, &bi, &ai)
        .await
        .unwrap();
    let bg = db::causal_genesis::install_pristine_genesis(&b.db, &user, &bi, &ba, &root, None)
        .await
        .unwrap()
        .local_id;
    db::group_db::delete_group(&a.db, &user, group.id)
        .await
        .unwrap();
    let GroupLifecycle::Deleted { deletions } =
        db::causal_group_lifecycle::get_state(&a.db, &user, &ai, group.id)
            .await
            .unwrap()
            .lifecycle
    else {
        panic!("expected deletion")
    };
    let deletion = db::causal_group_lifecycle::export_operation(&a.db, &user, &ai, &deletions[0])
        .await
        .unwrap();
    let deleted_item = db::sync_transport::encode_operation(&deletion).unwrap();
    let response = router
        .clone()
        .oneshot(publish(&aw, &deleted_item, &cookie, &aw.writer_secret))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let prior_id: i64 = sqlx::query_scalar(
        "SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(hub_user.id)
    .bind(&root.entity().global_id)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(
        db::group_db::purge_trashed_groups(&hub.db, 0)
            .await
            .unwrap(),
        1
    );
    db::group_db::edit_group(&b.db, &user, bg, "late keep", None, None)
        .await
        .unwrap();
    let NameState::Ready { token, .. } =
        db::causal_group_lifecycle::get_state(&b.db, &user, &bi, bg)
            .await
            .unwrap()
            .name
    else {
        panic!("expected late rename")
    };
    let rename =
        db::causal_group_lifecycle::export_operation(&b.db, &user, &bi, &token.frontier[0])
            .await
            .unwrap();
    let rename_item = db::sync_transport::encode_operation(&rename).unwrap();
    let response = router
        .clone()
        .oneshot(publish(&bw, &rename_item, &cookie, &bw.writer_secret))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "concurrent rename must keep the purged group"
    );
    let row: (i64,String,Option<String>) = sqlx::query_as("SELECT group_id,group_name,group_birth_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?").bind(hub_user.id).bind(&root.entity().global_id).fetch_one(&*hub.db).await.unwrap();
    assert_ne!(row.0, prior_id);
    assert_eq!(row.1, "late keep");
    assert_eq!(row.2, root.entity().birth_id);
    let records: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM group_deletion_records WHERE user_id=? AND group_global_id=?",
    )
    .bind(hub_user.id)
    .bind(&root.entity().global_id)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(records, 1, "purge must retain the deletion evidence");
    let retry = router
        .oneshot(publish(&aw, &deleted_item, &cookie, &aw.writer_secret))
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::OK);
    let retry: serde_json::Value =
        serde_json::from_slice(&to_bytes(retry.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(retry["status"], "already_present");
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Preserve both real competing authors through the transport.
async fn forged_name_resolution_cannot_choose_an_unoffered_name() {
    use db::causal_group_rename::{NameState, ResolveResult};
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_a_dir, a) = crate::test_support::app_state().await;
    let (_b_dir, b) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let ai = db::user_db::get_sync_identity(&a.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let bi = db::user_db::get_sync_identity(&b.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let aw = registration(&router, &cookie, &a).await;
    let bw = registration(&router, &cookie, &b).await;
    for (device, identity, writer) in [(&a, &ai, &aw), (&b, &bi, &bw)] {
        db::sync_peer::bind_peer(&device.db, &user, identity, &writer.server_identity)
            .await
            .unwrap();
        db::sync_transport::activate_writer(&device.db, &user, identity, writer)
            .await
            .unwrap();
    }
    let group = db::group_db::add_empty_group(&a.db, &user, "original", None)
        .await
        .unwrap();
    let root = db::causal_group_lifecycle::enroll_pristine_group(&a.db, &user, &ai, group.id)
        .await
        .unwrap();
    let response = router
        .clone()
        .oneshot(publish(
            &aw,
            &db::sync_transport::encode_genesis(&root).unwrap(),
            &cookie,
            &aw.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ba = db::sync_peer::bind_peer(&b.db, &user, &bi, &ai)
        .await
        .unwrap();
    let bg = db::causal_genesis::install_pristine_genesis(&b.db, &user, &bi, &ba, &root, None)
        .await
        .unwrap()
        .local_id;
    for (device, identity, id, name, writer) in [
        (&a, &ai, group.id, "alpha", &aw),
        (&b, &bi, bg, "bravo", &bw),
    ] {
        db::group_db::edit_group(&device.db, &user, id, name, None, None)
            .await
            .unwrap();
        let state = db::causal_group_lifecycle::get_state(&device.db, &user, identity, id)
            .await
            .unwrap();
        let NameState::Ready { token, .. } = state.name else {
            panic!("expected rename")
        };
        let op = db::causal_group_lifecycle::export_operation(
            &device.db,
            &user,
            identity,
            &token.frontier[0],
        )
        .await
        .unwrap();
        let response = router
            .clone()
            .oneshot(publish(
                writer,
                &db::sync_transport::encode_operation(&op).unwrap(),
                &cookie,
                &writer.writer_secret,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let NameState::Ready { token: bt, .. } =
        db::causal_group_lifecycle::get_state(&b.db, &user, &bi, bg)
            .await
            .unwrap()
            .name
    else {
        panic!("expected bravo")
    };
    let bop = db::causal_group_lifecycle::export_operation(&b.db, &user, &bi, &bt.frontier[0])
        .await
        .unwrap();
    let ab = db::sync_peer::bind_peer(&a.db, &user, &ai, &bi)
        .await
        .unwrap();
    db::causal_group_lifecycle::import_operation(&a.db, &user, &ai, &ab, &bop)
        .await
        .unwrap();
    let NameState::Conflict { token, .. } =
        db::causal_group_lifecycle::get_state(&a.db, &user, &ai, group.id)
            .await
            .unwrap()
            .name
    else {
        panic!("expected conflict")
    };
    let ResolveResult::Resolved(NameState::Ready { token, .. }) =
        db::causal_group_rename::resolve(&a.db, &user, &ai, group.id, &token, "alpha")
            .await
            .unwrap()
    else {
        panic!("expected offered resolution")
    };
    let resolution =
        db::causal_group_lifecycle::export_operation(&a.db, &user, &ai, &token.frontier[0])
            .await
            .unwrap();
    let valid = db::sync_transport::encode_operation(&resolution).unwrap();
    let mut forged = valid.clone();
    let wire = forged["envelopes"][0].as_str().unwrap();
    let mut event = db::causal_envelope::parse_causal_envelope(wire).unwrap();
    let db::causal_envelope::Action::GroupNameResolved { name } = &mut event.action else {
        panic!("expected resolution event")
    };
    *name = "unoffered".into();
    forged["envelopes"][0] = db::causal_envelope::canonical_causal_envelope(&event)
        .unwrap()
        .into();
    let before: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log)",
    )
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    let denied = router
        .clone()
        .oneshot(publish(&aw, &forged, &cookie, &aw.writer_secret))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "a forged unoffered resolution must not be acknowledged"
    );
    let after: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log)",
    )
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(after, before);
    let accepted = router
        .oneshot(publish(&aw, &valid, &cookie, &aw.writer_secret))
        .await
        .unwrap();
    assert_eq!(
        accepted.status(),
        StatusCode::OK,
        "an offered resolution must still succeed"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One end-to-end history verifies atomic retries across the same objects.
async fn complete_move_projects_both_membership_events_and_retries() {
    use db::causal_model_move::MoveState;
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
    let group = db::group_db::add_empty_group(&source.db, &user, "destination", None)
        .await
        .unwrap();
    let group_root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    let response = router
        .clone()
        .oneshot(publish(
            &writer,
            &db::sync_transport::encode_genesis(&group_root).unwrap(),
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let hub_identity = db::user_db::get_sync_identity(&hub.db, hub_user.id)
        .await
        .unwrap()
        .unwrap();
    let peer = db::sync_peer::bind_peer(&hub.db, &hub_user, &hub_identity, &identity)
        .await
        .unwrap();
    let hash = db::random_hex_32();
    let source_blob = db::blob_db::add_blob(&source.db, &hash, "stl", 1, None)
        .await
        .unwrap();
    let hub_blob = db::blob_db::add_blob(&hub.db, &hash, "stl", 1, None)
        .await
        .unwrap();
    let model = db::model_db::add_model(&source.db, &user, "part", source_blob, None, None)
        .await
        .unwrap();
    let model_root =
        db::causal_group_lifecycle::enroll_pristine_model(&source.db, &user, &identity, model)
            .await
            .unwrap();
    db::group_db::set_group_id_on_models(&source.db, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    let state = db::causal_model_move::get_state(&source.db, &user, &identity, model)
        .await
        .unwrap();
    let MoveState::Ready { token, .. } = state else {
        panic!("expected ready move")
    };
    let operation = db::causal_group_lifecycle::export_operation(
        &source.db,
        &user,
        &identity,
        &token.frontier[0],
    )
    .await
    .unwrap();
    assert_eq!(
        operation.envelopes().len(),
        2,
        "a move into a group has paired events"
    );
    let item = db::sync_transport::encode_operation(&operation).unwrap();
    let before_missing: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log)",
    )
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    let missing = router
        .clone()
        .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_ne!(
        missing.status(),
        StatusCode::OK,
        "missing model genesis cannot be acknowledged"
    );
    let after_missing: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log)",
    )
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(after_missing, before_missing);
    let target_model = db::causal_genesis::install_pristine_genesis(
        &hub.db,
        &hub_user,
        &hub_identity,
        &peer,
        &model_root,
        Some(hub_blob),
    )
    .await
    .unwrap()
    .local_id;
    let before: (i64, i64, Option<i64>) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log),model_group_id FROM models WHERE model_id=? AND model_user_id=?").bind(target_model).bind(hub_user.id).fetch_one(&*hub.db).await.unwrap();
    let mut truncated = item.clone();
    truncated["envelopes"].as_array_mut().unwrap().pop();
    let denied = router
        .clone()
        .oneshot(publish(&writer, &truncated, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "a truncated operation cannot be acknowledged"
    );
    let mut forged = item.clone();
    let events = forged["envelopes"].as_array_mut().unwrap();
    let mut event =
        db::causal_envelope::parse_causal_envelope(events[1].as_str().unwrap()).unwrap();
    event.event.writer_id = db::random_hex_32();
    events[1] = db::causal_envelope::canonical_causal_envelope(&event)
        .unwrap()
        .into();
    let denied = router
        .clone()
        .oneshot(publish(&writer, &forged, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "a second event with an unverified author cannot be acknowledged"
    );
    for mutation in ["reordered", "duplicate"] {
        let mut malformed = item.clone();
        let events = malformed["envelopes"].as_array_mut().unwrap();
        if mutation == "reordered" {
            events.reverse();
        } else {
            events.push(events[0].clone());
        }
        let denied = router
            .clone()
            .oneshot(publish(&writer, &malformed, &cookie, &writer.writer_secret))
            .await
            .unwrap();
        assert_ne!(
            denied.status(),
            StatusCode::OK,
            "{mutation} operation must not be acknowledged"
        );
    }
    let mut mismatched = item.clone();
    let events = mismatched["envelopes"].as_array_mut().unwrap();
    let mut add = db::causal_envelope::parse_causal_envelope(events[1].as_str().unwrap()).unwrap();
    assert!(matches!(
        add.action,
        db::causal_envelope::Action::GroupMemberAdded { .. }
    ));
    add.entity.global_id = db::random_hex_32();
    events[1] = db::causal_envelope::canonical_causal_envelope(&add)
        .unwrap()
        .into();
    let denied = router
        .clone()
        .oneshot(publish(
            &writer,
            &mismatched,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "an add for a different group cannot complete the move"
    );
    let mut explicit_null = item.clone();
    explicit_null["author_context"] = serde_json::Value::Null;
    let denied = router
        .clone()
        .oneshot(publish(
            &writer,
            &explicit_null,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "explicit null cannot masquerade as omitted legacy context"
    );
    let after: (i64, i64, Option<i64>) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log),model_group_id FROM models WHERE model_id=? AND model_user_id=?").bind(target_model).bind(hub_user.id).fetch_one(&*hub.db).await.unwrap();
    assert_eq!(
        after, before,
        "rejected partial or forged operations must not write or project"
    );
    sqlx::query("CREATE TRIGGER reject_combined_log BEFORE INSERT ON sync_transport_log BEGIN SELECT RAISE(ABORT,'injected log failure'); END").execute(&*hub.db).await.unwrap();
    let failed = router
        .clone()
        .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_ne!(
        failed.status(),
        StatusCode::OK,
        "a failed receipt must not acknowledge the move"
    );
    let after_failure: (i64, i64, Option<i64>) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log),model_group_id FROM models WHERE model_id=? AND model_user_id=?").bind(target_model).bind(hub_user.id).fetch_one(&*hub.db).await.unwrap();
    assert_eq!(
        after_failure, before,
        "receipt failure must roll back the complete move"
    );
    sqlx::query("DROP TRIGGER reject_combined_log")
        .execute(&*hub.db)
        .await
        .unwrap();
    let accepted = router
        .clone()
        .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_eq!(
        accepted.status(),
        StatusCode::OK,
        "the complete real move must be accepted"
    );
    let target_group: Option<i64> = sqlx::query_scalar(
        "SELECT model_group_id FROM models WHERE model_id=? AND model_user_id=?",
    )
    .bind(target_model)
    .bind(hub_user.id)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert!(
        target_group.is_some(),
        "the hub must project the membership"
    );
    let retry = router
        .clone()
        .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::OK);
    let result: serde_json::Value =
        serde_json::from_slice(&to_bytes(retry.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(result["status"], "already_present");

    // A second device moves the same model from the common ungrouped root.
    let (_other_dir, other) = crate::test_support::app_state().await;
    let other_identity = db::user_db::get_sync_identity(&other.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let other_writer = registration(&router, &cookie, &other).await;
    db::sync_peer::bind_peer(
        &other.db,
        &user,
        &other_identity,
        &other_writer.server_identity,
    )
    .await
    .unwrap();
    db::sync_transport::install_authority(&other.db, &user, &other_identity, &other_writer)
        .await
        .unwrap();
    let authority = db::sync_transport::load_authority(
        &other.db,
        &user,
        &other_identity,
        &other_writer.connection_id,
    )
    .await
    .unwrap();
    let other_peer = db::sync_peer::bind_peer(&other.db, &user, &other_identity, &identity)
        .await
        .unwrap();
    let other_blob = db::blob_db::add_blob(&other.db, &hash, "stl", 1, None)
        .await
        .unwrap();
    let other_model = db::causal_genesis::install_pristine_genesis(
        &other.db,
        &user,
        &other_identity,
        &other_peer,
        &model_root,
        Some(other_blob),
    )
    .await
    .unwrap()
    .local_id;
    let other_group = db::group_db::add_empty_group(&other.db, &user, "alternative", None)
        .await
        .unwrap();
    let other_root = db::causal_group_lifecycle::enroll_pristine_group(
        &other.db,
        &user,
        &other_identity,
        other_group.id,
    )
    .await
    .unwrap();
    let root_proof =
        db::sync_transport::writer_proof(&other.db, &user, &other_identity, other_root.event())
            .await
            .unwrap();
    let other_genesis = db::sync_transport::encode_genesis(&other_root).unwrap();
    let accepted = router
        .clone()
        .oneshot(publish_offline(
            &authority,
            &root_proof,
            &other_genesis,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    let other_binding = db::sync_peer::bind_peer(&source.db, &user, &identity, &other_identity)
        .await
        .unwrap();
    db::causal_genesis::install_pristine_genesis(
        &source.db,
        &user,
        &identity,
        &other_binding,
        &other_root,
        None,
    )
    .await
    .unwrap();
    db::group_db::set_group_id_on_models(
        &other.db,
        &user,
        Some(other_group.id),
        vec![other_model],
        None,
    )
    .await
    .unwrap();
    let MoveState::Ready {
        token: other_token, ..
    } = db::causal_model_move::get_state(&other.db, &user, &other_identity, other_model)
        .await
        .unwrap()
    else {
        panic!("expected other move")
    };
    let other_operation = db::causal_group_lifecycle::export_operation(
        &other.db,
        &user,
        &other_identity,
        &other_token.frontier[0],
    )
    .await
    .unwrap();
    let other_item = db::sync_transport::encode_operation(&other_operation).unwrap();
    let other_proof = db::sync_transport::writer_proof(
        &other.db,
        &user,
        &other_identity,
        &other_token.frontier[0],
    )
    .await
    .unwrap();
    let accepted = router
        .clone()
        .oneshot(publish_offline(
            &authority,
            &other_proof,
            &other_item,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    let relay = router
        .clone()
        .oneshot(publish(
            &writer,
            &other_item,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(
        relay.status(),
        StatusCode::OK,
        "exact relay must preserve the other writer's original event"
    );
    let state = db::causal_model_move::get_state(&hub.db, &hub_user, &hub_identity, target_model)
        .await
        .unwrap();
    assert!(
        matches!(state, MoveState::Conflict { .. }),
        "concurrent moves must require a choice"
    );
    db::causal_group_lifecycle::import_operation(
        &source.db,
        &user,
        &identity,
        &other_binding,
        &other_operation,
    )
    .await
    .unwrap();
    let chosen = Some(group_root.entity().clone());
    let outdated_choice =
        db::causal_model_move::resolve(&source.db, &user, &identity, model, &token, chosen.clone())
            .await
            .unwrap();
    assert!(
        matches!(
            outdated_choice,
            db::causal_model_move::MoveResolveResult::Stale(_)
        ),
        "stale displayed choices cannot resolve a newer conflict"
    );
    let MoveState::Conflict {
        token: conflict, ..
    } = db::causal_model_move::get_state(&source.db, &user, &identity, model)
        .await
        .unwrap()
    else {
        panic!("expected conflict")
    };
    let resolved =
        db::causal_model_move::resolve(&source.db, &user, &identity, model, &conflict, chosen)
            .await
            .unwrap();
    let db::causal_model_move::MoveResolveResult::Resolved(MoveState::Ready {
        token: resolved_token,
        ..
    }) = resolved
    else {
        panic!("expected resolved choice")
    };
    let resolution = db::causal_group_lifecycle::export_operation(
        &source.db,
        &user,
        &identity,
        &resolved_token.frontier[0],
    )
    .await
    .unwrap();
    let resolution_item = db::sync_transport::encode_operation(&resolution).unwrap();
    let accepted = router
        .clone()
        .oneshot(publish(
            &writer,
            &resolution_item,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(
        accepted.status(),
        StatusCode::OK,
        "the explicit choice must be delivered"
    );
    let chosen_group: Option<i64> = sqlx::query_scalar(
        "SELECT model_group_id FROM models WHERE model_id=? AND model_user_id=?",
    )
    .bind(target_model)
    .bind(hub_user.id)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(chosen_group, target_group);

    db::group_db::delete_group(&source.db, &user, group.id)
        .await
        .unwrap();
    let state = db::causal_group_lifecycle::get_state(&source.db, &user, &identity, group.id)
        .await
        .unwrap();
    let db::causal_group_lifecycle::GroupLifecycle::Deleted { deletions } = state.lifecycle else {
        panic!("expected deleted group")
    };
    let deletion =
        db::causal_group_lifecycle::export_operation(&source.db, &user, &identity, &deletions[0])
            .await
            .unwrap();
    assert!(
        deletion.envelopes().len() >= 2,
        "deletion of a populated group includes detachment"
    );
    // This rename was authored independently, after the source chose its delete cut.
    let other_original = db::causal_genesis::install_pristine_genesis(
        &other.db,
        &user,
        &other_identity,
        &other_peer,
        &group_root,
        None,
    )
    .await
    .unwrap()
    .local_id;
    db::group_db::edit_group(
        &other.db,
        &user,
        other_original,
        "concurrent rename",
        None,
        None,
    )
    .await
    .unwrap();
    let other_state =
        db::causal_group_lifecycle::get_state(&other.db, &user, &other_identity, other_original)
            .await
            .unwrap();
    let db::causal_group_rename::NameState::Ready {
        token: other_name, ..
    } = other_state.name
    else {
        panic!("expected concurrent rename")
    };
    let concurrent = db::causal_group_lifecycle::export_operation(
        &other.db,
        &user,
        &other_identity,
        &other_name.frontier[0],
    )
    .await
    .unwrap();
    let proof = db::sync_transport::writer_proof(
        &other.db,
        &user,
        &other_identity,
        &other_name.frontier[0],
    )
    .await
    .unwrap();
    let delivered = router
        .clone()
        .oneshot(publish_offline(
            &authority,
            &proof,
            &db::sync_transport::encode_operation(&concurrent).unwrap(),
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(delivered.status(), StatusCode::OK);
    db::group_db::set_group_id_on_models(&other.db, &user, None, vec![other_model], None)
        .await
        .unwrap();
    let independent =
        db::causal_model_move::get_state(&other.db, &user, &other_identity, other_model)
            .await
            .unwrap();
    let MoveState::Ready {
        token: independent_token,
        ..
    } = independent
    else {
        panic!("expected independent move")
    };
    let independent = db::causal_group_lifecycle::export_operation(
        &other.db,
        &user,
        &other_identity,
        &independent_token.frontier[0],
    )
    .await
    .unwrap();
    let proof = db::sync_transport::writer_proof(
        &other.db,
        &user,
        &other_identity,
        &independent_token.frontier[0],
    )
    .await
    .unwrap();
    let delivered = router
        .clone()
        .oneshot(publish_offline(
            &authority,
            &proof,
            &db::sync_transport::encode_operation(&independent).unwrap(),
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(delivered.status(), StatusCode::OK);
    let delete_item = db::sync_transport::encode_operation(&deletion).unwrap();
    let context = delete_item["author_context"]
        .as_str()
        .expect("new deletion must carry its authored causal cut");
    let decoded: serde_json::Value = serde_json::from_str(context).unwrap();
    assert_eq!(
        canonical_context(&decoded),
        context,
        "forged contexts must use the canonical wire encoding"
    );
    assert_eq!(decoded["version"], 1);
    assert_ne!(
        decoded["frontier"].as_array().unwrap().as_slice(),
        [] as [serde_json::Value; 0]
    );
    assert_eq!(
        decoded["events"].as_array().unwrap().len(),
        deletion.envelopes().len()
    );
    let mut historical = delete_item.clone();
    historical.as_object_mut().unwrap().remove("author_context");
    let denied = router
        .clone()
        .oneshot(publish(
            &writer,
            &historical,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "historical lifecycle operations without authored context cannot be admitted"
    );
    for mutation in ["wrong", "duplicate"] {
        let mut dots: Vec<db::causal_envelope::Dot> =
            serde_json::from_value(decoded["events"].clone()).unwrap();
        let original_dot = serde_json::to_string(&dots[1]).unwrap();
        if mutation == "wrong" {
            dots[1].writer_id = db::random_hex_32();
        } else {
            dots[1] = dots[0].clone();
        }
        let replacement = serde_json::to_string(&dots[1]).unwrap();
        let offset = context.rfind(&original_dot).unwrap();
        let mut changed = context.to_owned();
        changed.replace_range(offset..offset + original_dot.len(), &replacement);
        let mut forged = delete_item.clone();
        forged["author_context"] = changed.into();
        let denied = router
            .clone()
            .oneshot(publish(&writer, &forged, &cookie, &writer.writer_secret))
            .await
            .unwrap();
        assert_ne!(
            denied.status(),
            StatusCode::OK,
            "{mutation} declared dots cannot describe otherwise intact events"
        );
    }
    let mut incomplete = delete_item.clone();
    incomplete["envelopes"].as_array_mut().unwrap().pop();
    let denied = router
        .clone()
        .oneshot(publish(
            &writer,
            &incomplete,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "incomplete deletion must not be acknowledged"
    );
    let mut falsified = incomplete.clone();
    let mut reduced_context = decoded.clone();
    reduced_context["events"].as_array_mut().unwrap().pop();
    falsified["author_context"] = canonical_context(&reduced_context).into();
    let denied = router
        .clone()
        .oneshot(publish(&writer, &falsified, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "omitting a detach cannot be hidden by changing declared event dots"
    );
    let frontier = reduced_context["frontier"].as_array_mut().unwrap();
    frontier.retain(|entry| entry["entity"]["kind"] == "group");
    assert!(
        !frontier.is_empty(),
        "forgery retains an authored group frontier"
    );
    falsified["author_context"] = canonical_context(&reduced_context).into();
    let denied = router
        .clone()
        .oneshot(publish(&writer, &falsified, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "a group frontier alone must not hide an omitted model"
    );
    reduced_context["frontier"] = serde_json::json!([]);
    falsified["author_context"] = canonical_context(&reduced_context).into();
    let denied = router
        .clone()
        .oneshot(publish(&writer, &falsified, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "a missing authored frontier must not hide omitted members"
    );
    let mut extra = delete_item.clone();
    let payload = extra["envelopes"].as_array_mut().unwrap();
    payload.push(payload[1].clone());
    let mut extra_context = decoded.clone();
    let dots = extra_context["events"].as_array_mut().unwrap();
    dots.push(dots[1].clone());
    extra["author_context"] = canonical_context(&extra_context).into();
    let denied = router
        .clone()
        .oneshot(publish(&writer, &extra, &cookie, &writer.writer_secret))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "an extra duplicated member event must not be acknowledged"
    );
    let still_assigned: Option<i64> = sqlx::query_scalar(
        "SELECT model_group_id FROM models WHERE model_id=? AND model_user_id=?",
    )
    .bind(target_model)
    .bind(hub_user.id)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(
        still_assigned, target_group,
        "incomplete deletion must not detach models"
    );
    let before_failure: (i64, i64, i64, i64, Option<i64>) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log),(SELECT COUNT(*) FROM group_deletion_records),(SELECT COUNT(*) FROM group_lifecycle_operations WHERE author_context IS NOT NULL),model_group_id FROM models WHERE model_id=? AND model_user_id=?").bind(target_model).bind(hub_user.id).fetch_one(&*hub.db).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_delete_log BEFORE INSERT ON sync_transport_log BEGIN SELECT RAISE(ABORT,'injected delete log failure'); END").execute(&*hub.db).await.unwrap();
    let failed = router
        .clone()
        .oneshot(publish(
            &writer,
            &delete_item,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_ne!(
        failed.status(),
        StatusCode::OK,
        "failed delivery log cannot acknowledge a deletion"
    );
    let after_failure: (i64, i64, i64, i64, Option<i64>) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log),(SELECT COUNT(*) FROM group_deletion_records),(SELECT COUNT(*) FROM group_lifecycle_operations WHERE author_context IS NOT NULL),model_group_id FROM models WHERE model_id=? AND model_user_id=?").bind(target_model).bind(hub_user.id).fetch_one(&*hub.db).await.unwrap();
    assert_eq!(
        after_failure, before_failure,
        "context, tombstone and projection must roll back together"
    );
    sqlx::query("DROP TRIGGER reject_delete_log")
        .execute(&*hub.db)
        .await
        .unwrap();
    let accepted = router
        .clone()
        .oneshot(publish(
            &writer,
            &delete_item,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(
        accepted.status(),
        StatusCode::OK,
        "a complete authenticated deletion must be accepted"
    );
    let detached: Option<i64> = sqlx::query_scalar(
        "SELECT model_group_id FROM models WHERE model_id=? AND model_user_id=?",
    )
    .bind(target_model)
    .bind(hub_user.id)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(
        detached, target_group,
        "a concurrent rename keeps the group and its model"
    );
    let conflict =
        db::causal_model_move::get_state(&hub.db, &hub_user, &hub_identity, target_model)
            .await
            .unwrap();
    assert!(
        matches!(conflict, MoveState::Conflict { choices, .. } if choices.iter().any(|choice| choice.destination.is_none())),
        "the independent ungroup must remain an explicit choice"
    );
    let active: Option<String> = sqlx::query_scalar(
        "SELECT group_deleted_at FROM models_group WHERE group_id=? AND group_user_id=?",
    )
    .bind(target_group.unwrap())
    .bind(hub_user.id)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(active, None, "the concurrent rename must keep the group");
    let repeated = router
        .clone()
        .clone()
        .oneshot(publish(
            &writer,
            &delete_item,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(repeated.status(), StatusCode::OK);
    let repeated: serde_json::Value =
        serde_json::from_slice(&to_bytes(repeated.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(repeated["status"], "already_present");

    let deleted = db::group_db::get_trashed_groups(&source.db, &user, 30)
        .await
        .unwrap()
        .into_iter()
        .find(|entry| entry.id == group.id)
        .unwrap();
    let request = db::group_db::GroupRestoreRequest {
        id: deleted.id,
        group_global_id: deleted.group_global_id,
        deletion_operation_id: deleted.deletion_operation_id.unwrap(),
    };
    db::group_db::restore_groups(&source.db, &user, &[request], 30)
        .await
        .unwrap();
    let state = db::causal_group_lifecycle::get_state(&source.db, &user, &identity, group.id)
        .await
        .unwrap();
    let db::causal_group_rename::NameState::Ready { token, .. } = state.name else {
        panic!("expected restored group")
    };
    let restore = db::causal_group_lifecycle::export_operation(
        &source.db,
        &user,
        &identity,
        &token.frontier[0],
    )
    .await
    .unwrap();
    assert!(
        restore.envelopes().len() >= 3,
        "restoring a former member requires model and group events"
    );
    let restore_item = db::sync_transport::encode_operation(&restore).unwrap();
    let restore_context: serde_json::Value = serde_json::from_str(
        restore_item["author_context"]
            .as_str()
            .expect("restore must carry authored context"),
    )
    .unwrap();
    let before_restore: (i64, i64, Option<i64>) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log),model_group_id FROM models WHERE model_id=? AND model_user_id=?").bind(target_model).bind(hub_user.id).fetch_one(&*hub.db).await.unwrap();
    let mut incomplete = restore_item.clone();
    incomplete["envelopes"].as_array_mut().unwrap().truncate(1);
    let mut forged_context = restore_context.clone();
    forged_context["events"].as_array_mut().unwrap().truncate(1);
    incomplete["author_context"] = canonical_context(&forged_context).into();
    let denied = router
        .clone()
        .oneshot(publish(
            &writer,
            &incomplete,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "omitting restore and add cannot be hidden by changing declared event dots"
    );
    let after_restore: (i64, i64, Option<i64>) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM sync_transport_log),model_group_id FROM models WHERE model_id=? AND model_user_id=?").bind(target_model).bind(hub_user.id).fetch_one(&*hub.db).await.unwrap();
    assert_eq!(
        after_restore, before_restore,
        "incomplete restore must not modify hub or receipt"
    );
    let accepted = router
        .clone()
        .oneshot(publish(
            &writer,
            &restore_item,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(
        accepted.status(),
        StatusCode::OK,
        "complete authenticated restore must be accepted"
    );
    let restored: Option<i64> = sqlx::query_scalar(
        "SELECT model_group_id FROM models WHERE model_id=? AND model_user_id=?",
    )
    .bind(target_model)
    .bind(hub_user.id)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(
        restored, target_group,
        "an ungrouped former member is restored"
    );
    let repeated = router
        .clone()
        .oneshot(publish(
            &writer,
            &restore_item,
            &cookie,
            &writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(repeated.status(), StatusCode::OK);
    let repeated: serde_json::Value =
        serde_json::from_slice(&to_bytes(repeated.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(repeated["status"], "already_present");
    let path = format!(
        "/api/v2/sync/items?protocol=1&connection_id={}&after=0&limit=64",
        writer.connection_id
    );
    let request = Request::get(path)
        .header(header::COOKIE, &cookie)
        .header("x-sync-writer-secret", &writer.writer_secret)
        .body(Body::empty())
        .unwrap();
    let page = router.oneshot(request).await.unwrap();
    assert_eq!(
        page.status(),
        StatusCode::OK,
        "all stored receipts must reconstruct exactly"
    );
    let page: serde_json::Value =
        serde_json::from_slice(&to_bytes(page.into_body(), 2 * 1024 * 1024).await.unwrap())
            .unwrap();
    let items = page["items"].as_array().unwrap();
    assert!(
        items.iter().any(|entry| entry["item"] == delete_item),
        "deletion context and original dots must survive delivery"
    );
    assert!(
        items.iter().any(|entry| entry["item"] == restore_item),
        "restore context and original dots must survive delivery"
    );
}
