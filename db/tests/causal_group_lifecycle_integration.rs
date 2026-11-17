//! Explicit combined-capability tests; these do not enable network sync.
mod common;
use db::{
    blob_db, causal_genesis,
    causal_group_lifecycle::{self, GroupLifecycle, OperationImportResult},
    causal_group_rename::NameState,
    db_context::DbContext,
    group_db,
    model::user::User,
    model_db, random_hex_32, sync_peer, user_db,
    user_db::SyncIdentity,
};

struct Device {
    _dir: tempfile::TempDir,
    db: DbContext,
    user: User,
    identity: SyncIdentity,
    group: i64,
    model: i64,
}
async fn devices() -> (Device, Device, Device) {
    let (ad, a) = common::test_db().await;
    let (bd, b) = common::test_db().await;
    let (cd, c) = common::test_db().await;
    let user = User::default();
    let ai = user_db::get_sync_identity(&a, user.id)
        .await
        .unwrap()
        .unwrap();
    let bi = user_db::get_sync_identity(&b, user.id)
        .await
        .unwrap()
        .unwrap();
    let ci = user_db::get_sync_identity(&c, user.id)
        .await
        .unwrap()
        .unwrap();
    let ba = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let ca = sync_peer::bind_peer(&c, &user, &ci, &ai).await.unwrap();
    let group = group_db::add_empty_group(&a, &user, "original", None)
        .await
        .unwrap();
    let gb = causal_group_lifecycle::enroll_pristine_group(&a, &user, &ai, group.id).await;
    assert!(
        gb.is_ok(),
        "fresh group must enroll in combined lifecycle scope: {gb:?}"
    );
    let gb = gb.unwrap();
    let bg = causal_genesis::install_pristine_genesis(&b, &user, &bi, &ba, &gb, None)
        .await
        .unwrap()
        .local_id;
    let cg = causal_genesis::install_pristine_genesis(&c, &user, &ci, &ca, &gb, None)
        .await
        .unwrap()
        .local_id;
    let hash = random_hex_32();
    let ablob = blob_db::add_blob(&a, &hash, "stl", 10, None).await.unwrap();
    let bblob = blob_db::add_blob(&b, &hash, "stl", 10, None).await.unwrap();
    let cblob = blob_db::add_blob(&c, &hash, "stl", 10, None).await.unwrap();
    let model = model_db::add_model(&a, &user, "part", ablob, None, None)
        .await
        .unwrap();
    let mb = causal_group_lifecycle::enroll_pristine_model(&a, &user, &ai, model)
        .await
        .unwrap();
    let bm = causal_genesis::install_pristine_genesis(&b, &user, &bi, &ba, &mb, Some(bblob))
        .await
        .unwrap()
        .local_id;
    let cm = causal_genesis::install_pristine_genesis(&c, &user, &ci, &ca, &mb, Some(cblob))
        .await
        .unwrap()
        .local_id;
    (
        Device {
            _dir: ad,
            db: a,
            user: user.clone(),
            identity: ai,
            group: group.id,
            model,
        },
        Device {
            _dir: bd,
            db: b,
            user: user.clone(),
            identity: bi,
            group: bg,
            model: bm,
        },
        Device {
            _dir: cd,
            db: c,
            user,
            identity: ci,
            group: cg,
            model: cm,
        },
    )
}
async fn state(d: &Device) -> causal_group_lifecycle::GroupState {
    causal_group_lifecycle::get_state(&d.db, &d.user, &d.identity, d.group)
        .await
        .unwrap()
}
async fn latest(d: &Device) -> db::causal_envelope::Dot {
    match state(d).await.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready name frontier: {x:?}"),
    }
}
async fn transfer(
    target: &Device,
    source: &Device,
    operation: &causal_group_lifecycle::OperationBundle,
) {
    let peer = sync_peer::bind_peer(&target.db, &target.user, &target.identity, &source.identity)
        .await
        .unwrap();
    let result = causal_group_lifecycle::import_operation(
        &target.db,
        &target.user,
        &target.identity,
        &peer,
        operation,
    )
    .await;
    assert!(
        result.is_ok(),
        "verified operation with complete dependencies must import: {result:?}"
    );
    let result = result.unwrap();
    assert!(
        matches!(result, OperationImportResult::Applied(_)),
        "complete dependencies must apply: {result:?}"
    );
}

#[tokio::test]
#[allow(clippy::many_single_char_names, clippy::too_many_lines)]
async fn concurrent_delete_and_rename_keep_group_model_and_durable_deletion() {
    use db::causal_envelope::Action;
    let (a, b, c) = devices().await;
    let h = group_db::add_empty_group(&a.db, &a.user, "destination", None)
        .await
        .unwrap();
    let h_genesis =
        causal_group_lifecycle::enroll_pristine_group(&a.db, &a.user, &a.identity, h.id)
            .await
            .unwrap();
    let ba = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    let ca = sync_peer::bind_peer(&c.db, &c.user, &c.identity, &a.identity)
        .await
        .unwrap();
    let bh = causal_genesis::install_pristine_genesis(
        &b.db,
        &b.user,
        &b.identity,
        &ba,
        &h_genesis,
        None,
    )
    .await
    .unwrap()
    .local_id;
    let ch = causal_genesis::install_pristine_genesis(
        &c.db,
        &c.user,
        &c.identity,
        &ca,
        &h_genesis,
        None,
    )
    .await
    .unwrap()
    .local_id;
    let moved =
        group_db::set_group_id_on_models(&a.db, &a.user, Some(a.group), vec![a.model], None).await;
    assert!(
        moved.is_ok(),
        "verified active destination must accept the move: {moved:?}"
    );
    let addition =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    assert!(
        addition
            .envelopes()
            .iter()
            .any(|e| matches!(e.action, Action::ModelMoved { .. }))
    );
    assert!(
        addition
            .envelopes()
            .iter()
            .any(|e| matches!(e.action, Action::GroupMemberAdded { .. }))
    );
    transfer(&b, &a, &addition).await;
    transfer(&c, &a, &addition).await;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    let deletion =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    group_db::edit_group(&b.db, &b.user, b.group, "renamed", None, None)
        .await
        .unwrap();
    let rename =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    transfer(&a, &b, &rename).await;
    transfer(&b, &a, &deletion).await;
    transfer(&c, &a, &deletion).await;
    transfer(&c, &b, &rename).await;
    for d in [&a, &b, &c] {
        let observed = state(d).await;
        assert!(matches!(observed.lifecycle, GroupLifecycle::Active));
        assert!(matches!(observed.name,NameState::Ready{ref name,..} if name=="renamed"));
        let row:(String,Option<String>,Option<i64>)=sqlx::query_as("SELECT g.group_name,g.group_deleted_at,m.model_group_id FROM models_group g JOIN models m ON m.model_id=? WHERE g.group_id=?").bind(d.model).bind(d.group).fetch_one(&d.db).await.unwrap();
        assert_eq!(row, ("renamed".into(), None, Some(d.group)));
        assert!(
            !group_db::get_trashed_groups(&d.db, &d.user, 30)
                .await
                .unwrap()
                .iter()
                .any(|g| g.id == d.group)
        );
        let deleted: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM group_deletion_records WHERE user_id=? AND group_global_id=?",
        )
        .bind(d.user.id)
        .bind(&observed.entity.global_id)
        .fetch_one(&d.db)
        .await
        .unwrap();
        assert!(deleted > 0, "losing deletion evidence must remain durable");
    }
    group_db::set_group_id_on_models(&a.db, &a.user, Some(h.id), vec![a.model], None)
        .await
        .unwrap();
    let h_state = causal_group_lifecycle::get_state(&a.db, &a.user, &a.identity, h.id)
        .await
        .unwrap();
    let h_dot = match h_state.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected destination frontier: {x:?}"),
    };
    let moved = causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &h_dot)
        .await
        .unwrap();
    transfer(&b, &a, &moved).await;
    transfer(&c, &a, &moved).await;
    for (d, target) in [(&a, h.id), (&b, bh), (&c, ch)] {
        let destination: Option<i64> =
            sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
                .bind(d.model)
                .fetch_one(&d.db)
                .await
                .unwrap();
        assert_eq!(
            destination,
            Some(target),
            "a later independent move must survive delete reconciliation"
        );
    }
}

#[tokio::test]
async fn concurrent_delete_and_new_membership_keep_group_and_added_model() {
    let (a, b, c) = devices().await;
    let hash = random_hex_32();
    let ablob = blob_db::add_blob(&a.db, &hash, "stl", 5, None)
        .await
        .unwrap();
    let bblob = blob_db::add_blob(&b.db, &hash, "stl", 5, None)
        .await
        .unwrap();
    let cblob = blob_db::add_blob(&c.db, &hash, "stl", 5, None)
        .await
        .unwrap();
    let model = model_db::add_model(&a.db, &a.user, "new member", ablob, None, None)
        .await
        .unwrap();
    let genesis = causal_group_lifecycle::enroll_pristine_model(&a.db, &a.user, &a.identity, model)
        .await
        .unwrap();
    let ba = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    let ca = sync_peer::bind_peer(&c.db, &c.user, &c.identity, &a.identity)
        .await
        .unwrap();
    let bm = causal_genesis::install_pristine_genesis(
        &b.db,
        &b.user,
        &b.identity,
        &ba,
        &genesis,
        Some(bblob),
    )
    .await
    .unwrap()
    .local_id;
    let cm = causal_genesis::install_pristine_genesis(
        &c.db,
        &c.user,
        &c.identity,
        &ca,
        &genesis,
        Some(cblob),
    )
    .await
    .unwrap()
    .local_id;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    let deletion =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    group_db::set_group_id_on_models(&b.db, &b.user, Some(b.group), vec![bm], None)
        .await
        .unwrap();
    let addition =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    transfer(&a, &b, &addition).await;
    transfer(&b, &a, &deletion).await;
    transfer(&c, &b, &addition).await;
    transfer(&c, &a, &deletion).await;
    for (device, local_model) in [(&a, model), (&b, bm), (&c, cm)] {
        assert!(matches!(
            state(device).await.lifecycle,
            GroupLifecycle::Active
        ));
        let destination: Option<i64> =
            sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
                .bind(local_model)
                .fetch_one(&device.db)
                .await
                .unwrap();
        assert_eq!(destination, Some(device.group));
        assert!(
            !group_db::get_trashed_groups(&device.db, &device.user, 30)
                .await
                .unwrap()
                .iter()
                .any(|g| g.id == device.group)
        );
    }
}

#[tokio::test]
async fn operation_failure_after_first_event_rolls_back_whole_move() {
    let (a, _b, _c) = devices().await;
    let before:(Option<i64>,i64,i64,i64)=sqlx::query_as("SELECT model_group_id,(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM group_lifecycle_events) FROM models WHERE model_id=?").bind(a.model).fetch_one(&a.db).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_second_combined_event BEFORE INSERT ON group_lifecycle_events WHEN (SELECT COUNT(*) FROM group_lifecycle_events)=1 BEGIN SELECT RAISE(ABORT,'injected second combined event'); END").execute(&a.db).await.unwrap();
    let error =
        group_db::set_group_id_on_models(&a.db, &a.user, Some(a.group), vec![a.model], None)
            .await
            .unwrap_err();
    assert!(error.to_string().contains("injected second combined event"));
    let after:(Option<i64>,i64,i64,i64)=sqlx::query_as("SELECT model_group_id,(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM group_lifecycle_events) FROM models WHERE model_id=?").bind(a.model).fetch_one(&a.db).await.unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn missing_group_parent_requests_dependency_without_ack_then_retry_and_relay() {
    let (a, b, c) = devices().await;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.group), vec![a.model], None)
        .await
        .unwrap();
    let parent_dot = latest(&a).await;
    let parent = causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &parent_dot)
        .await
        .unwrap();
    group_db::edit_group(&a.db, &a.user, a.group, "renamed", None, None)
        .await
        .unwrap();
    let child_dot = latest(&a).await;
    let child = causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &child_dot)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    let before = state(&b).await;
    let result =
        causal_group_lifecycle::import_operation(&b.db, &b.user, &b.identity, &peer, &child).await;
    assert!(
        result.is_ok(),
        "missing parent must produce a dependency request: {result:?}"
    );
    let result = result.unwrap();
    assert!(
        matches!(result,OperationImportResult::NeedsDependencies{ref missing,ref state} if missing.iter().any(|d|d.event==parent_dot) && *state==before)
    );
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM causal_events WHERE writer_id=? AND counter=?)",
    )
    .bind(&child_dot.writer_id)
    .bind(child_dot.counter.parse::<i64>().unwrap())
    .fetch_one(&b.db)
    .await
    .unwrap();
    assert!(
        !present,
        "dependency request is not an event acknowledgement"
    );
    transfer(&b, &a, &parent).await;
    transfer(&b, &a, &child).await;
    let after = state(&b).await;
    transfer(&b, &a, &child).await;
    assert_eq!(
        state(&b).await,
        after,
        "exact retry cannot add a new action"
    );
    let relayed_parent =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &parent_dot)
            .await
            .unwrap();
    let relayed_child =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &child_dot)
            .await
            .unwrap();
    transfer(&c, &b, &relayed_parent).await;
    transfer(&c, &b, &relayed_child).await;
    let relayed = causal_group_lifecycle::export_operation(&c.db, &c.user, &c.identity, &child_dot)
        .await
        .unwrap();
    let before_a = state(&a).await;
    transfer(&a, &c, &relayed).await;
    assert_eq!(
        state(&a).await,
        before_a,
        "relayed original action back to author must be an exact retry"
    );
}

#[tokio::test]
async fn manual_trash_restore_relays_exact_deletion_and_reattaches_former_member() {
    let (a, b, c) = devices().await;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.group), vec![a.model], None)
        .await
        .unwrap();
    let addition =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    transfer(&b, &a, &addition).await;
    transfer(&c, &a, &addition).await;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    let deleted =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    transfer(&b, &a, &deleted).await;
    transfer(&c, &a, &deleted).await;
    for d in [&a, &b, &c] {
        assert!(matches!(
            state(d).await.lifecycle,
            GroupLifecycle::Deleted { .. }
        ));
    }
    let request = common::group_restore_request(&b.db, &b.user, b.group).await;
    group_db::restore_groups(&b.db, &b.user, std::slice::from_ref(&request), 30)
        .await
        .unwrap();
    let restore =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    transfer(&a, &b, &restore).await;
    transfer(&c, &b, &restore).await;
    for d in [&a, &b, &c] {
        assert!(matches!(state(d).await.lifecycle, GroupLifecycle::Active));
        let destination: Option<i64> =
            sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
                .bind(d.model)
                .fetch_one(&d.db)
                .await
                .unwrap();
        assert_eq!(destination, Some(d.group));
    }
    let before:(i64,i64)=sqlx::query_as("SELECT (SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM group_lifecycle_events)").fetch_one(&b.db).await.unwrap();
    let retry = group_db::restore_groups(&b.db, &b.user, std::slice::from_ref(&request), 30).await;
    assert!(
        retry.is_ok(),
        "exact retry of the same restore must be a no-op: {retry:?}"
    );
    let after:(i64,i64)=sqlx::query_as("SELECT (SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM group_lifecycle_events)").fetch_one(&b.db).await.unwrap();
    assert_eq!(after, before);
    group_db::delete_group(&b.db, &b.user, b.group)
        .await
        .unwrap();
    assert!(
        group_db::restore_groups(&b.db, &b.user, &[request], 30)
            .await
            .is_err(),
        "old restore must not target a later deletion"
    );
}

#[tokio::test]
#[allow(clippy::many_single_char_names)]
async fn restoring_group_does_not_reattach_a_model_independently_moved_after_delete() {
    let (a, b, c) = devices().await;
    let h = group_db::add_empty_group(&a.db, &a.user, "destination", None)
        .await
        .unwrap();
    let genesis = causal_group_lifecycle::enroll_pristine_group(&a.db, &a.user, &a.identity, h.id)
        .await
        .unwrap();
    let ba = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    let ca = sync_peer::bind_peer(&c.db, &c.user, &c.identity, &a.identity)
        .await
        .unwrap();
    let bh =
        causal_genesis::install_pristine_genesis(&b.db, &b.user, &b.identity, &ba, &genesis, None)
            .await
            .unwrap()
            .local_id;
    let ch =
        causal_genesis::install_pristine_genesis(&c.db, &c.user, &c.identity, &ca, &genesis, None)
            .await
            .unwrap()
            .local_id;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.group), vec![a.model], None)
        .await
        .unwrap();
    let added =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    transfer(&b, &a, &added).await;
    transfer(&c, &a, &added).await;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    let deleted =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    transfer(&b, &a, &deleted).await;
    transfer(&c, &a, &deleted).await;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(h.id), vec![a.model], None)
        .await
        .unwrap();
    let h_state = causal_group_lifecycle::get_state(&a.db, &a.user, &a.identity, h.id)
        .await
        .unwrap();
    let dot = match h_state.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected destination frontier: {x:?}"),
    };
    let moved = causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &dot)
        .await
        .unwrap();
    transfer(&b, &a, &moved).await;
    transfer(&c, &a, &moved).await;
    let request = common::group_restore_request(&b.db, &b.user, b.group).await;
    let result = group_db::restore_groups(&b.db, &b.user, &[request], 30).await;
    assert!(
        result.is_ok(),
        "restore must retain an independently moved model: {result:?}"
    );
    let restored =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    transfer(&a, &b, &restored).await;
    transfer(&c, &b, &restored).await;
    for (d, target) in [(&a, h.id), (&b, bh), (&c, ch)] {
        assert!(matches!(state(d).await.lifecycle, GroupLifecycle::Active));
        let current: Option<i64> =
            sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
                .bind(d.model)
                .fetch_one(&d.db)
                .await
                .unwrap();
        assert_eq!(
            current,
            Some(target),
            "restore must not overwrite an independent later move"
        );
    }
}

#[tokio::test]
async fn delete_observing_prior_rename_and_add_stays_deleted() {
    let (a, b, c) = devices().await;
    group_db::set_group_id_on_models(&b.db, &b.user, Some(b.group), vec![b.model], None)
        .await
        .unwrap();
    let add =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    transfer(&a, &b, &add).await;
    transfer(&c, &b, &add).await;
    group_db::edit_group(&b.db, &b.user, b.group, "renamed", None, None)
        .await
        .unwrap();
    let rename =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    transfer(&a, &b, &rename).await;
    transfer(&c, &b, &rename).await;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    let deletion =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    transfer(&b, &a, &deletion).await;
    transfer(&c, &a, &deletion).await;
    for d in [&a, &b, &c] {
        assert!(matches!(
            state(d).await.lifecycle,
            GroupLifecycle::Deleted { .. }
        ));
        let current: Option<i64> =
            sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
                .bind(d.model)
                .fetch_one(&d.db)
                .await
                .unwrap();
        assert_eq!(current, None);
        assert!(
            group_db::get_trashed_groups(&d.db, &d.user, 30)
                .await
                .unwrap()
                .iter()
                .any(|g| g.id == d.group)
        );
    }
}

#[tokio::test]
async fn combined_capacity_exhaustion_keeps_local_rename_usable_and_retains_history() {
    use db::causal_envelope::{self, Action, CausalEnvelope, Dot};
    let (a, _b, _c) = devices().await;
    let original = state(&a).await;
    let root = match &original.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready {x:?}"),
    };
    let birth = original.entity.birth_id.as_ref().unwrap();
    let writer = random_hex_32();
    let operation = random_hex_32();
    let mut parent = root;
    let mut tx = a.db.begin().await.unwrap();
    sqlx::query("INSERT INTO group_lifecycle_operations(user_id,operation_id,focus_birth_id,event_count) VALUES (?,?,?,?)").bind(a.user.id).bind(&operation).bind(birth).bind(i64::try_from(db::causal_store::MAX_OWNER_EVENTS).unwrap()).execute(&mut *tx).await.unwrap();
    for n in 1..=db::causal_store::MAX_OWNER_EVENTS {
        let dot = Dot {
            writer_id: writer.clone(),
            counter: n.to_string(),
        };
        let event = CausalEnvelope {
            version: 2,
            entity: original.entity.clone(),
            event: dot.clone(),
            parents: vec![parent],
            dependencies: vec![],
            action: Action::GroupRenamed {
                name: "original".into(),
            },
        };
        sqlx::query(
            "INSERT INTO causal_events(user_id,writer_id,counter,envelope) VALUES (?,?,?,?)",
        )
        .bind(a.user.id)
        .bind(&writer)
        .bind(i64::try_from(n).unwrap())
        .bind(causal_envelope::canonical_causal_envelope(&event).unwrap())
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query("INSERT INTO group_lifecycle_events(user_id,operation_id,position,writer_id,counter) VALUES (?,?,?,?,?)").bind(a.user.id).bind(&operation).bind(i64::try_from(n-1).unwrap()).bind(&writer).bind(i64::try_from(n).unwrap()).execute(&mut *tx).await.unwrap();
        parent = dot;
    }
    tx.commit().await.unwrap();
    let edited =
        group_db::edit_group(&a.db, &a.user, a.group, "local after capacity", None, None).await;
    assert!(
        edited.is_ok(),
        "combined history capacity must not block local rename: {edited:?}"
    );
    let current: String =
        sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=?")
            .bind(a.group)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert_eq!(current, "local after capacity");
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM group_lifecycle_events WHERE user_id=?")
            .bind(a.user.id)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert_eq!(
        usize::try_from(remaining).unwrap(),
        db::causal_store::MAX_OWNER_EVENTS
    );
    assert!(
        matches!(state(&a).await.name, NameState::Paused { .. }),
        "capacity fallback must expose paused status"
    );
}

#[tokio::test]
async fn combined_rename_and_move_honor_supplied_timestamps() {
    let (a, _b, _c) = devices().await;
    let timestamp = "2001-02-03T04:05:06Z";
    group_db::edit_group(&a.db, &a.user, a.group, "renamed", Some(timestamp), None)
        .await
        .unwrap();
    let group_time: String =
        sqlx::query_scalar("SELECT group_last_modified FROM models_group WHERE group_id=?")
            .bind(a.group)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert_eq!(
        group_time, timestamp,
        "combined rename must honor supplied timestamp"
    );
    let move_timestamp = "2002-03-04T05:06:07Z";
    group_db::set_group_id_on_models(
        &a.db,
        &a.user,
        Some(a.group),
        vec![a.model],
        Some(move_timestamp),
    )
    .await
    .unwrap();
    let group_time: String =
        sqlx::query_scalar("SELECT group_last_modified FROM models_group WHERE group_id=?")
            .bind(a.group)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert_eq!(
        group_time, move_timestamp,
        "combined move must update its destination group's timestamp"
    );
}

#[tokio::test]
async fn resolving_offered_ungrouped_choice_preserves_delete_dependencies() {
    use db::causal_model_move::{self, MoveResolveResult, MoveState};
    let (a, b, _c) = devices().await;
    let h = group_db::add_empty_group(&a.db, &a.user, "destination", None)
        .await
        .unwrap();
    let genesis = causal_group_lifecycle::enroll_pristine_group(&a.db, &a.user, &a.identity, h.id)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    let _bh = causal_genesis::install_pristine_genesis(
        &b.db,
        &b.user,
        &b.identity,
        &peer,
        &genesis,
        None,
    )
    .await
    .unwrap()
    .local_id;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.group), vec![a.model], None)
        .await
        .unwrap();
    let initial =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    transfer(&b, &a, &initial).await;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a.db, &a.user, Some(h.id), vec![a.model], None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&b.db, &b.user, None, vec![b.model], None)
        .await
        .unwrap();
    let model_state = causal_model_move::get_state(&b.db, &b.user, &b.identity, b.model)
        .await
        .unwrap();
    let dot = match model_state {
        MoveState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected model frontier: {x:?}"),
    };
    let moved = causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &dot)
        .await
        .unwrap();
    transfer(&a, &b, &moved).await;
    let current = causal_model_move::get_state(&a.db, &a.user, &a.identity, a.model)
        .await
        .unwrap();
    let token = match current {
        MoveState::Conflict { token, choices } => {
            assert!(choices.iter().any(|c| c.destination.is_none()));
            token
        }
        x => panic!("expected detach versus independent move conflict: {x:?}"),
    };
    let result =
        causal_model_move::resolve(&a.db, &a.user, &a.identity, a.model, &token, None).await;
    assert!(
        matches!(
            result,
            Ok(MoveResolveResult::Resolved(MoveState::Ready {
                destination: None,
                ..
            }))
        ),
        "offered ungrouped resolution must preserve deletion dependencies: {result:?}"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn deleting_visible_nonfrontier_group_detaches_cached_assignment_and_retains_choices() {
    use db::causal_model_move::{self, MoveState};
    let (a, b, c) = devices().await;
    let mut ids = Vec::new();
    for name in ["destination H", "destination J"] {
        let g = group_db::add_empty_group(&a.db, &a.user, name, None)
            .await
            .unwrap();
        let genesis =
            causal_group_lifecycle::enroll_pristine_group(&a.db, &a.user, &a.identity, g.id)
                .await
                .unwrap();
        let ba = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
            .await
            .unwrap();
        let ca = sync_peer::bind_peer(&c.db, &c.user, &c.identity, &a.identity)
            .await
            .unwrap();
        let bg = causal_genesis::install_pristine_genesis(
            &b.db,
            &b.user,
            &b.identity,
            &ba,
            &genesis,
            None,
        )
        .await
        .unwrap()
        .local_id;
        let cg = causal_genesis::install_pristine_genesis(
            &c.db,
            &c.user,
            &c.identity,
            &ca,
            &genesis,
            None,
        )
        .await
        .unwrap()
        .local_id;
        ids.push((g.id, bg, cg));
    }
    group_db::set_group_id_on_models(&b.db, &b.user, Some(b.group), vec![b.model], None)
        .await
        .unwrap();
    let original =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    transfer(&c, &b, &original).await;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(ids[0].0), vec![a.model], None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&c.db, &c.user, Some(ids[1].2), vec![c.model], None)
        .await
        .unwrap();
    let a_state = causal_group_lifecycle::get_state(&a.db, &a.user, &a.identity, ids[0].0)
        .await
        .unwrap();
    let a_dot = match a_state.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready: {x:?}"),
    };
    let c_state = causal_group_lifecycle::get_state(&c.db, &c.user, &c.identity, ids[1].2)
        .await
        .unwrap();
    let c_dot = match c_state.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready: {x:?}"),
    };
    let a_op = causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &a_dot)
        .await
        .unwrap();
    let c_op = causal_group_lifecycle::export_operation(&c.db, &c.user, &c.identity, &c_dot)
        .await
        .unwrap();
    transfer(&b, &a, &a_op).await;
    transfer(&b, &c, &c_op).await;
    let before = causal_model_move::get_state(&b.db, &b.user, &b.identity, b.model)
        .await
        .unwrap();
    let choices = match before {
        MoveState::Conflict { choices, .. } => choices,
        x => panic!("expected conflict: {x:?}"),
    };
    let visible: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(b.model)
            .fetch_one(&b.db)
            .await
            .unwrap();
    assert_eq!(visible, Some(b.group));
    group_db::delete_group(&b.db, &b.user, b.group)
        .await
        .unwrap();
    let visible: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(b.model)
            .fetch_one(&b.db)
            .await
            .unwrap();
    assert_eq!(
        visible, None,
        "deleted visible destination must not retain its cached assignment"
    );
    let after = causal_model_move::get_state(&b.db, &b.user, &b.identity, b.model)
        .await
        .unwrap();
    assert!(
        matches!(after,MoveState::Conflict{choices:ref retained,..} | MoveState::Paused{choices:ref retained,..} if *retained==choices),
        "unresolved destination choices must remain readable: {after:?}"
    );
}

#[tokio::test]
async fn combined_name_choice_rejects_stale_frontier_after_third_branch() {
    use db::causal_group_rename::{self, ResolveResult};
    let (a, b, c) = devices().await;
    group_db::edit_group(&a.db, &a.user, a.group, "alpha", None, None)
        .await
        .unwrap();
    group_db::edit_group(&b.db, &b.user, b.group, "bravo", None, None)
        .await
        .unwrap();
    group_db::edit_group(&c.db, &c.user, c.group, "charlie", None, None)
        .await
        .unwrap();
    let be =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    let ce =
        causal_group_lifecycle::export_operation(&c.db, &c.user, &c.identity, &latest(&c).await)
            .await
            .unwrap();
    transfer(&a, &b, &be).await;
    let old = match state(&a).await.name {
        NameState::Conflict { token, .. } => token,
        x => panic!("expected conflict {x:?}"),
    };
    let before_choice = state(&a).await;
    let event_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events")
        .fetch_one(&a.db)
        .await
        .unwrap();
    assert!(
        causal_group_rename::resolve(&a.db, &a.user, &a.identity, a.group, &old, "unoffered")
            .await
            .is_err(),
        "an unoffered name must not resolve a conflict"
    );
    assert_eq!(state(&a).await, before_choice);
    let after_choice: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events")
        .fetch_one(&a.db)
        .await
        .unwrap();
    assert_eq!(after_choice, event_count);
    transfer(&a, &c, &ce).await;
    let current = state(&a).await;
    assert!(matches!(current.name,NameState::Conflict{ref choices,..} if choices.len()==3));
    let before:(String,i64,i64)=sqlx::query_as("SELECT group_name,(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM causal_events) FROM models_group WHERE group_id=?").bind(a.group).fetch_one(&a.db).await.unwrap();
    let result = causal_group_rename::resolve(&a.db, &a.user, &a.identity, a.group, &old, "alpha")
        .await
        .unwrap();
    assert!(matches!(result, ResolveResult::Stale(_)));
    let after:(String,i64,i64)=sqlx::query_as("SELECT group_name,(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM causal_events) FROM models_group WHERE group_id=?").bind(a.group).fetch_one(&a.db).await.unwrap();
    assert_eq!(after, before);
    assert_eq!(state(&a).await, current);
}

#[tokio::test]
async fn generic_receipt_reads_moves_targeting_rename_and_member_added_lifecycle() {
    use db::{causal_envelope::Action, causal_store};
    let (a, b, _c) = devices().await;
    group_db::edit_group(&a.db, &a.user, a.group, "renamed", None, None)
        .await
        .unwrap();
    let renamed =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    transfer(&b, &a, &renamed).await;
    let moved =
        group_db::set_group_id_on_models(&a.db, &a.user, Some(a.group), vec![a.model], None).await;
    assert!(
        moved.is_ok(),
        "a renamed active group must remain a valid destination: {moved:?}"
    );
    let first =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    let first_dot = first
        .envelopes()
        .iter()
        .find(|event| matches!(event.action, Action::ModelMoved { .. }))
        .unwrap()
        .event
        .clone();
    transfer(&b, &a, &first).await;
    let first_read = causal_store::get_received(&b.db, &b.user, &b.identity, &first_dot).await;
    assert!(
        first_read.is_ok(),
        "generic receipt must accept a checked group rename as move target"
    );
    assert!(first_read.unwrap().is_some());
    let hash = random_hex_32();
    let source_blob = blob_db::add_blob(&a.db, &hash, "stl", 3, None)
        .await
        .unwrap();
    let target_blob = blob_db::add_blob(&b.db, &hash, "stl", 3, None)
        .await
        .unwrap();
    let model = model_db::add_model(&a.db, &a.user, "second", source_blob, None, None)
        .await
        .unwrap();
    let genesis = causal_group_lifecycle::enroll_pristine_model(&a.db, &a.user, &a.identity, model)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    causal_genesis::install_pristine_genesis(
        &b.db,
        &b.user,
        &b.identity,
        &peer,
        &genesis,
        Some(target_blob),
    )
    .await
    .unwrap();
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.group), vec![model], None)
        .await
        .unwrap();
    let second =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    let second_dot = second
        .envelopes()
        .iter()
        .find(|event| matches!(event.action, Action::ModelMoved { .. }))
        .unwrap()
        .event
        .clone();
    transfer(&b, &a, &second).await;
    let second_read = causal_store::get_received(&b.db, &b.user, &b.identity, &second_dot).await;
    assert!(
        second_read.is_ok(),
        "generic receipt must accept a checked member-add as move target"
    );
    assert!(second_read.unwrap().is_some());
}

#[tokio::test]
async fn generic_receipt_rejects_delete_as_active_move_target() {
    use db::{
        causal_envelope::{self, Action, CausalEnvelope, Dependency, Dot, Entity, EntityKind},
        causal_store,
        group_history::ReplicaPrincipal,
    };
    let (a, b, _c) = devices().await;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    let deletion =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    let deleted = deletion
        .envelopes()
        .iter()
        .find(|event| matches!(event.action, Action::GroupDeleted { .. }))
        .unwrap();
    transfer(&b, &a, &deletion).await;
    let (global, birth): (String, String) =
        sqlx::query_as("SELECT model_unique_global_id,model_birth_id FROM models WHERE model_id=?")
            .bind(b.model)
            .fetch_one(&b.db)
            .await
            .unwrap();
    let writer = random_hex_32();
    sqlx::query("INSERT INTO causal_replica_owners(user_id,replica_id) VALUES (?,?)")
        .bind(b.user.id)
        .bind(&writer)
        .execute(&b.db)
        .await
        .unwrap();
    let principal = ReplicaPrincipal {
        owner_user_id: b.user.id,
        replica_id: writer.clone(),
    };
    let event = CausalEnvelope {
        version: 2,
        entity: Entity {
            kind: EntityKind::Model,
            global_id: global,
            birth_id: Some(birth),
        },
        event: Dot {
            writer_id: writer,
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![Dependency {
            entity: deleted.entity.clone(),
            event: deleted.event.clone(),
        }],
        action: Action::ModelMoved {
            from: None,
            to: Some(deleted.entity.clone()),
            target_lifecycle: Some(deleted.event.clone()),
        },
    };
    let result = causal_store::receive(
        &b.db,
        &b.user,
        &b.identity,
        &principal,
        &causal_envelope::canonical_causal_envelope(&event).unwrap(),
    )
    .await;
    assert!(result.is_err(), "a delete cannot prove an active target");
}

#[tokio::test]
async fn combined_enrollment_limit_rejects_extra_scope_without_blocking_existing_edits() {
    use db::causal_envelope::{self, Action, CausalEnvelope, Dot, Entity, EntityKind};
    let (a, _b, _c) = devices().await;
    let extra = group_db::add_empty_group(&a.db, &a.user, "extra", None)
        .await
        .unwrap();
    let (materialization,): (String,)=sqlx::query_as("SELECT materialization FROM causal_genesis_bindings WHERE user_id=? AND entity_kind='group' AND local_id=?").bind(a.user.id).bind(a.group).fetch_one(&a.db).await.unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM group_lifecycle_scopes WHERE user_id=?")
            .bind(a.user.id)
            .fetch_one(&a.db)
            .await
            .unwrap();
    let writer = random_hex_32();
    let mut tx = a.db.begin().await.unwrap();
    for n in 0..(i64::try_from(db::causal_store::MAX_OWNER_EVENTS).unwrap() - count) {
        let global = random_hex_32();
        let birth = random_hex_32();
        let id=sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id,group_birth_id) VALUES ('original','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?,?)").bind(a.user.id).bind(&global).bind(&birth).execute(&mut *tx).await.unwrap().last_insert_rowid();
        let event = CausalEnvelope {
            version: 2,
            entity: Entity {
                kind: EntityKind::Group,
                global_id: global.clone(),
                birth_id: Some(birth.clone()),
            },
            event: Dot {
                writer_id: writer.clone(),
                counter: (n + 1).to_string(),
            },
            parents: vec![],
            dependencies: vec![],
            action: Action::GroupCreated {
                name: "original".into(),
            },
        };
        sqlx::query(
            "INSERT INTO causal_events(user_id,writer_id,counter,envelope) VALUES (?,?,?,?)",
        )
        .bind(a.user.id)
        .bind(&writer)
        .bind(n + 1)
        .bind(causal_envelope::canonical_causal_envelope(&event).unwrap())
        .execute(&mut *tx)
        .await
        .unwrap();
        let guard:String=sqlx::query_scalar("SELECT json_object('kind','group','generation',h.generation,'head',h.head,'revision',g.group_revision) FROM models_group g JOIN group_sync_heads h ON h.user_id=g.group_user_id AND h.group_global_id=g.group_unique_global_id WHERE g.group_id=?").bind(id).fetch_one(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO causal_genesis_bindings(user_id,entity_kind,global_id,birth_id,writer_id,counter,local_instance_id,local_account_id,source_instance_id,source_account_id,local_id,local_guard,materialization,origin,capability) VALUES (?,'group',?,?,?,?,?,?,?,?,?,?,?,'local','group_lifecycle_v1')").bind(a.user.id).bind(&global).bind(&birth).bind(&writer).bind(n+1).bind(&a.identity.instance_id).bind(&a.identity.account_id).bind(&a.identity.instance_id).bind(&a.identity.account_id).bind(id).bind(guard).bind(&materialization).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    let before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM group_lifecycle_scopes WHERE user_id=?")
            .bind(a.user.id)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert_eq!(
        usize::try_from(before).unwrap(),
        db::causal_store::MAX_OWNER_EVENTS
    );
    let extra_result =
        causal_group_lifecycle::enroll_pristine_group(&a.db, &a.user, &a.identity, extra.id).await;
    assert!(
        extra_result.is_err(),
        "4097th scope must be rejected before it can block unrelated editing"
    );
    let after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM group_lifecycle_scopes WHERE user_id=?")
            .bind(a.user.id)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert_eq!(after, before);
    let edit = group_db::edit_group(&a.db, &a.user, a.group, "still usable", None, None).await;
    assert!(
        edit.is_ok(),
        "existing scoped edits must remain usable at capacity: {edit:?}"
    );
}

#[tokio::test]
async fn combined_enrollment_byte_limit_rejects_extra_scope_without_writes() {
    use db::causal_envelope::{self, Action, CausalEnvelope, Dot, Entity, EntityKind};
    let (a, _b, _c) = devices().await;
    let name = "x".repeat(causal_envelope::MAX_NAME_BYTES);
    let extra = group_db::add_empty_group(&a.db, &a.user, &name, None)
        .await
        .unwrap();
    let materialization=serde_json::json!({"kind":"group","name":name,"created":"2020-01-01T00:00:00Z","last_modified":"2020-01-01T00:00:00Z"}).to_string();
    let writer = random_hex_32();
    let mut tx = a.db.begin().await.unwrap();
    let scopes = (db::causal_store::MAX_OWNER_BYTES / (2 * causal_envelope::MAX_NAME_BYTES)) - 1;
    for n in 0..scopes {
        let global = random_hex_32();
        let birth = random_hex_32();
        let id=sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id,group_birth_id) VALUES (?,'2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?,?)").bind(&name).bind(a.user.id).bind(&global).bind(&birth).execute(&mut *tx).await.unwrap().last_insert_rowid();
        let event = CausalEnvelope {
            version: 2,
            entity: Entity {
                kind: EntityKind::Group,
                global_id: global.clone(),
                birth_id: Some(birth.clone()),
            },
            event: Dot {
                writer_id: writer.clone(),
                counter: (n + 1).to_string(),
            },
            parents: vec![],
            dependencies: vec![],
            action: Action::GroupCreated { name: name.clone() },
        };
        sqlx::query(
            "INSERT INTO causal_events(user_id,writer_id,counter,envelope) VALUES (?,?,?,?)",
        )
        .bind(a.user.id)
        .bind(&writer)
        .bind(i64::try_from(n + 1).unwrap())
        .bind(causal_envelope::canonical_causal_envelope(&event).unwrap())
        .execute(&mut *tx)
        .await
        .unwrap();
        let guard:String=sqlx::query_scalar("SELECT json_object('kind','group','generation',h.generation,'head',h.head,'revision',g.group_revision) FROM models_group g JOIN group_sync_heads h ON h.user_id=g.group_user_id AND h.group_global_id=g.group_unique_global_id WHERE g.group_id=?").bind(id).fetch_one(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO causal_genesis_bindings(user_id,entity_kind,global_id,birth_id,writer_id,counter,local_instance_id,local_account_id,source_instance_id,source_account_id,local_id,local_guard,materialization,origin,capability) VALUES (?,'group',?,?,?,?,?,?,?,?,?,?,?,'local','group_lifecycle_v1')").bind(a.user.id).bind(&global).bind(&birth).bind(&writer).bind(i64::try_from(n+1).unwrap()).bind(&a.identity.instance_id).bind(&a.identity.account_id).bind(&a.identity.instance_id).bind(&a.identity.account_id).bind(id).bind(guard).bind(&materialization).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    let before:(i64,i64)=sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(length(CAST(e.envelope AS BLOB))+length(CAST(b.materialization AS BLOB))+length(CAST(b.local_guard AS BLOB))),0) FROM causal_genesis_bindings b JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.capability='group_lifecycle_v1'").bind(a.user.id).fetch_one(&a.db).await.unwrap();
    assert!(usize::try_from(before.1).unwrap() < db::causal_store::MAX_OWNER_BYTES);
    let result =
        causal_group_lifecycle::enroll_pristine_group(&a.db, &a.user, &a.identity, extra.id).await;
    assert!(
        result.is_err(),
        "additional long-name root must be rejected at byte cap"
    );
    let after:(i64,i64)=sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(length(CAST(e.envelope AS BLOB))+length(CAST(b.materialization AS BLOB))+length(CAST(b.local_guard AS BLOB))),0) FROM causal_genesis_bindings b JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.capability='group_lifecycle_v1'").bind(a.user.id).fetch_one(&a.db).await.unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn expired_deleted_group_is_rehydrated_for_offline_concurrent_rename() {
    let (a, b, c) = devices().await;
    let original = state(&a).await.entity;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.group), vec![a.model], None)
        .await
        .unwrap();
    let added =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    transfer(&b, &a, &added).await;
    transfer(&c, &a, &added).await;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    let expired_request = common::group_restore_request(&a.db, &a.user, a.group).await;
    let deletion =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    group_db::edit_group(&b.db, &b.user, b.group, "offline rename", None, None)
        .await
        .unwrap();
    let rename =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(group_db::purge_trashed_groups(&a.db, 0).await.unwrap(), 1);
    assert!(
        group_db::restore_groups(&a.db, &a.user, &[expired_request], 30)
            .await
            .is_err(),
        "purge must not fabricate an expired manual recovery snapshot"
    );
    let gone: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models_group WHERE group_id=?)")
            .bind(a.group)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert!(!gone);
    let records: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM group_deletion_records WHERE user_id=? AND group_global_id=?",
    )
    .bind(a.user.id)
    .bind(&original.global_id)
    .fetch_one(&a.db)
    .await
    .unwrap();
    assert!(records > 0, "purge must retain the delete fact");
    transfer(&b, &a, &deletion).await;
    transfer(&c, &a, &deletion).await;
    transfer(&c, &b, &rename).await;
    let peer = sync_peer::bind_peer(&a.db, &a.user, &a.identity, &b.identity)
        .await
        .unwrap();
    let result =
        causal_group_lifecycle::import_operation(&a.db, &a.user, &a.identity, &peer, &rename).await;
    assert!(
        result.is_ok(),
        "offline rename must keep the original group after retention expiry: {result:?}"
    );
    assert!(matches!(result.unwrap(), OperationImportResult::Applied(_)));
    let rows:Vec<(i64,Option<String>)>=sqlx::query_as("SELECT group_id,group_deleted_at FROM models_group WHERE group_user_id=? AND group_unique_global_id=? AND group_birth_id=?").bind(a.user.id).bind(&original.global_id).bind(&original.birth_id).fetch_all(&a.db).await.unwrap();
    assert_eq!(rows.len(), 1);
    let (new_id, deleted) = &rows[0];
    assert_ne!(*new_id, a.group);
    assert_eq!(*deleted, None);
    let other_id = user_db::add_user(
        &a.db,
        "other",
        "retained-other@example.test",
        &random_hex_32(),
    )
    .await
    .unwrap();
    let other = user_db::get_user_by_id(&a.db, other_id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = user_db::get_sync_identity(&a.db, other_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        causal_group_lifecycle::get_state(&a.db, &other, &other_identity, *new_id)
            .await
            .is_err(),
        "rehydrated group remains owner scoped"
    );
    assert!(matches!(
        causal_group_lifecycle::get_state(&a.db, &a.user, &a.identity, *new_id)
            .await
            .unwrap()
            .lifecycle,
        GroupLifecycle::Active
    ));
    let member: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(a.model)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert_eq!(member, Some(*new_id));
    assert!(
        !group_db::get_trashed_groups(&a.db, &a.user, 30)
            .await
            .unwrap()
            .iter()
            .any(|x| x.group_global_id == original.global_id)
    );
    let after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM group_deletion_records WHERE user_id=? AND group_global_id=?",
    )
    .bind(a.user.id)
    .bind(&original.global_id)
    .fetch_one(&a.db)
    .await
    .unwrap();
    assert!(after >= records);
}

#[tokio::test]
async fn expired_deleted_group_is_rehydrated_for_offline_concurrent_add() {
    let (a, b, c) = devices().await;
    let original = state(&a).await.entity;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    let deletion =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    group_db::set_group_id_on_models(&b.db, &b.user, Some(b.group), vec![b.model], None)
        .await
        .unwrap();
    let addition =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(group_db::purge_trashed_groups(&a.db, 0).await.unwrap(), 1);
    transfer(&b, &a, &deletion).await;
    transfer(&c, &b, &addition).await;
    transfer(&c, &a, &deletion).await;
    let peer = sync_peer::bind_peer(&a.db, &a.user, &a.identity, &b.identity)
        .await
        .unwrap();
    let result =
        causal_group_lifecycle::import_operation(&a.db, &a.user, &a.identity, &peer, &addition)
            .await;
    assert!(
        matches!(result, Ok(OperationImportResult::Applied(_))),
        "offline model addition must keep original group after expiry: {result:?}"
    );
    let new_id:i64=sqlx::query_scalar("SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=? AND group_birth_id=? AND group_deleted_at IS NULL").bind(a.user.id).bind(&original.global_id).bind(&original.birth_id).fetch_one(&a.db).await.unwrap();
    assert_ne!(new_id, a.group);
    let member: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(a.model)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert_eq!(member, Some(new_id));
    let records: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM group_deletion_records WHERE user_id=? AND group_global_id=?",
    )
    .bind(a.user.id)
    .bind(&original.global_id)
    .fetch_one(&a.db)
    .await
    .unwrap();
    assert!(records > 0);
    assert!(
        !group_db::get_trashed_groups(&a.db, &a.user, 30)
            .await
            .unwrap()
            .iter()
            .any(|x| x.group_global_id == original.global_id)
    );
}

#[tokio::test]
#[allow(clippy::many_single_char_names)]
async fn expired_group_keep_does_not_reclaim_member_independently_moved_elsewhere() {
    let (a, b, c) = devices().await;
    let original = state(&a).await.entity;
    let h = group_db::add_empty_group(&a.db, &a.user, "destination", None)
        .await
        .unwrap();
    let genesis = causal_group_lifecycle::enroll_pristine_group(&a.db, &a.user, &a.identity, h.id)
        .await
        .unwrap();
    let ba = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    let ca = sync_peer::bind_peer(&c.db, &c.user, &c.identity, &a.identity)
        .await
        .unwrap();
    let bh =
        causal_genesis::install_pristine_genesis(&b.db, &b.user, &b.identity, &ba, &genesis, None)
            .await
            .unwrap()
            .local_id;
    let ch =
        causal_genesis::install_pristine_genesis(&c.db, &c.user, &c.identity, &ca, &genesis, None)
            .await
            .unwrap()
            .local_id;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.group), vec![a.model], None)
        .await
        .unwrap();
    let initial =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    transfer(&b, &a, &initial).await;
    transfer(&c, &a, &initial).await;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    let deletion =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    group_db::set_group_id_on_models(&a.db, &a.user, Some(h.id), vec![a.model], None)
        .await
        .unwrap();
    let destination_state = causal_group_lifecycle::get_state(&a.db, &a.user, &a.identity, h.id)
        .await
        .unwrap();
    let dot = match destination_state.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected destination frontier {x:?}"),
    };
    let moved = causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &dot)
        .await
        .unwrap();
    group_db::edit_group(&b.db, &b.user, b.group, "offline rename", None, None)
        .await
        .unwrap();
    let renamed =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(group_db::purge_trashed_groups(&a.db, 0).await.unwrap(), 1);
    transfer(&b, &a, &deletion).await;
    transfer(&b, &a, &moved).await;
    transfer(&c, &a, &deletion).await;
    transfer(&c, &a, &moved).await;
    transfer(&c, &b, &renamed).await;
    let peer = sync_peer::bind_peer(&a.db, &a.user, &a.identity, &b.identity)
        .await
        .unwrap();
    let result =
        causal_group_lifecycle::import_operation(&a.db, &a.user, &a.identity, &peer, &renamed)
            .await;
    assert!(
        matches!(result, Ok(OperationImportResult::Applied(_))),
        "offline keep must apply after purge: {result:?}"
    );
    for (d, target) in [(&a, h.id), (&b, bh), (&c, ch)] {
        let current: Option<i64> =
            sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
                .bind(d.model)
                .fetch_one(&d.db)
                .await
                .unwrap();
        assert_eq!(
            current,
            Some(target),
            "rehydration must not fabricate recovery or undo the independent move"
        );
    }
    let present:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models_group WHERE group_user_id=? AND group_unique_global_id=? AND group_birth_id=? AND group_deleted_at IS NULL)").bind(a.user.id).bind(&original.global_id).bind(&original.birth_id).fetch_one(&a.db).await.unwrap();
    assert!(present);
}

#[tokio::test]
async fn failed_rehydration_rolls_back_import_and_retains_purged_certificate() {
    let (a, b, _c) = devices().await;
    let original = state(&a).await.entity;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    group_db::edit_group(&b.db, &b.user, b.group, "offline", None, None)
        .await
        .unwrap();
    let rename =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(group_db::purge_trashed_groups(&a.db, 0).await.unwrap(), 1);
    sqlx::query("CREATE TRIGGER reject_group_rehydration BEFORE UPDATE ON group_lifecycle_group_rows WHEN OLD.state='purged' AND NEW.state='present' BEGIN SELECT RAISE(ABORT,'injected rehydration failure'); END").execute(&a.db).await.unwrap();
    let before:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group WHERE group_user_id=?),(SELECT COUNT(*) FROM causal_events WHERE user_id=?),(SELECT COUNT(*) FROM group_lifecycle_events WHERE user_id=?),(SELECT COUNT(*) FROM group_deletion_records WHERE user_id=?)").bind(a.user.id).bind(a.user.id).bind(a.user.id).bind(a.user.id).fetch_one(&a.db).await.unwrap();
    let peer = sync_peer::bind_peer(&a.db, &a.user, &a.identity, &b.identity)
        .await
        .unwrap();
    let error =
        causal_group_lifecycle::import_operation(&a.db, &a.user, &a.identity, &peer, &rename)
            .await
            .unwrap_err();
    assert!(
        error.to_string().contains("injected rehydration failure"),
        "failure must occur at the rehydration write: {error}"
    );
    let after:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group WHERE group_user_id=?),(SELECT COUNT(*) FROM causal_events WHERE user_id=?),(SELECT COUNT(*) FROM group_lifecycle_events WHERE user_id=?),(SELECT COUNT(*) FROM group_deletion_records WHERE user_id=?)").bind(a.user.id).bind(a.user.id).bind(a.user.id).bind(a.user.id).fetch_one(&a.db).await.unwrap();
    assert_eq!(after, before);
    let row:(String,i64)=sqlx::query_as("SELECT state,local_group_id FROM group_lifecycle_group_rows WHERE user_id=? AND birth_id=?").bind(a.user.id).bind(original.birth_id).fetch_one(&a.db).await.unwrap();
    assert_eq!(row, ("purged".into(), a.group));
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn repeated_purge_keep_preserves_birth_invalidates_old_token_and_retries_genesis() {
    use db::causal_group_rename::{self, ResolveResult};
    let (a, b, c) = devices().await;
    let entity = state(&a).await.entity;
    let genesis =
        causal_group_lifecycle::enroll_pristine_group(&a.db, &a.user, &a.identity, a.group)
            .await
            .unwrap();
    group_db::delete_group(&b.db, &b.user, b.group)
        .await
        .unwrap();
    let deletion =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    group_db::edit_group(&a.db, &a.user, a.group, "offline one", None, None)
        .await
        .unwrap();
    let rename =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(group_db::purge_trashed_groups(&b.db, 0).await.unwrap(), 1);
    transfer(&c, &b, &deletion).await;
    transfer(&c, &a, &rename).await;
    transfer(&b, &a, &rename).await;
    let first:i64=sqlx::query_scalar("SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=? AND group_birth_id=?").bind(b.user.id).bind(&entity.global_id).bind(&entity.birth_id).fetch_one(&b.db).await.unwrap();
    assert_ne!(first, b.group);
    let peer = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    let retry = causal_genesis::install_pristine_genesis(
        &b.db,
        &b.user,
        &b.identity,
        &peer,
        &genesis,
        None,
    )
    .await
    .unwrap();
    assert!(retry.already_present);
    assert_eq!(retry.local_id, first);
    let old_token = match causal_group_lifecycle::get_state(&b.db, &b.user, &b.identity, first)
        .await
        .unwrap()
        .name
    {
        NameState::Ready { token, .. } => token,
        x => panic!("expected ready: {x:?}"),
    };
    group_db::delete_group(&b.db, &b.user, first).await.unwrap();
    group_db::edit_group(&c.db, &c.user, c.group, "offline two", None, None)
        .await
        .unwrap();
    let rename2 =
        causal_group_lifecycle::export_operation(&c.db, &c.user, &c.identity, &latest(&c).await)
            .await
            .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(group_db::purge_trashed_groups(&b.db, 0).await.unwrap(), 1);
    transfer(&b, &c, &rename2).await;
    let second:i64=sqlx::query_scalar("SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=? AND group_birth_id=?").bind(b.user.id).bind(&entity.global_id).bind(&entity.birth_id).fetch_one(&b.db).await.unwrap();
    assert_ne!(second, first);
    let stale = causal_group_rename::resolve(
        &b.db,
        &b.user,
        &b.identity,
        second,
        &old_token,
        "offline one",
    )
    .await;
    assert!(
        stale.is_err() || matches!(stale, Ok(ResolveResult::Stale(_))),
        "old row's token cannot resolve the replacement"
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(b.user.id)
    .bind(&entity.global_id)
    .fetch_one(&b.db)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn failed_purge_certificate_write_rolls_back_group_and_head() {
    let (a, _b, _c) = devices().await;
    let entity = state(&a).await.entity;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let before:(i64,String,i64,i64)=sqlx::query_as("SELECT h.head,h.state,(SELECT COUNT(*) FROM models_group WHERE group_id=?),(SELECT COUNT(*) FROM group_deletion_records WHERE user_id=?) FROM group_sync_heads h WHERE h.user_id=? AND h.group_global_id=?").bind(a.group).bind(a.user.id).bind(a.user.id).bind(&entity.global_id).fetch_one(&a.db).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_purge_certificate BEFORE UPDATE ON group_lifecycle_group_rows WHEN NEW.state='purged' BEGIN SELECT RAISE(ABORT,'injected purge certificate failure'); END").execute(&a.db).await.unwrap();
    let result = group_db::purge_trashed_groups(&a.db, 0).await;
    assert!(
        result.is_err(),
        "a rejected certificate must abort the purge"
    );
    let error = result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("injected purge certificate failure")
    );
    let after:(i64,String,i64,i64)=sqlx::query_as("SELECT h.head,h.state,(SELECT COUNT(*) FROM models_group WHERE group_id=?),(SELECT COUNT(*) FROM group_deletion_records WHERE user_id=?) FROM group_sync_heads h WHERE h.user_id=? AND h.group_global_id=?").bind(a.group).bind(a.user.id).bind(a.user.id).bind(&entity.global_id).fetch_one(&a.db).await.unwrap();
    assert_eq!(after, before);
    assert!(
        group_db::get_trashed_groups(&a.db, &a.user, 30)
            .await
            .unwrap()
            .iter()
            .any(|entry| entry.id == a.group)
    );
}

#[tokio::test]
async fn late_keep_cannot_adopt_same_global_id_with_a_different_birth() {
    let (a, b, _c) = devices().await;
    let entity = state(&a).await.entity;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    group_db::edit_group(&b.db, &b.user, b.group, "offline", None, None)
        .await
        .unwrap();
    let rename =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(group_db::purge_trashed_groups(&a.db, 0).await.unwrap(), 1);
    let replacement_birth = random_hex_32();
    let replacement=sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id,group_birth_id) VALUES ('different','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?,?)").bind(a.user.id).bind(&entity.global_id).bind(&replacement_birth).execute(&a.db).await.unwrap().last_insert_rowid();
    let peer = sync_peer::bind_peer(&a.db, &a.user, &a.identity, &b.identity)
        .await
        .unwrap();
    assert!(
        causal_group_lifecycle::import_operation(&a.db, &a.user, &a.identity, &peer, &rename)
            .await
            .is_err()
    );
    let row: (String, String) =
        sqlx::query_as("SELECT group_name,group_birth_id FROM models_group WHERE group_id=?")
            .bind(replacement)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert_eq!(row, ("different".into(), replacement_birth));
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(a.user.id)
    .bind(&entity.global_id)
    .fetch_one(&a.db)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn purged_group_keep_rejects_replaced_account_with_reused_numeric_id() {
    let (a, b, _c) = devices().await;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    group_db::edit_group(&b.db, &b.user, b.group, "offline", None, None)
        .await
        .unwrap();
    let rename =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    let peer = sync_peer::bind_peer(&a.db, &a.user, &a.identity, &b.identity)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(group_db::purge_trashed_groups(&a.db, 0).await.unwrap(), 1);
    sqlx::query("DELETE FROM users WHERE user_id=?")
        .bind(a.user.id)
        .execute(&a.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (user_id,user_name,user_email,user_password_hash,user_created_at) VALUES (?,'replacement','retained-replacement@example.test','unused','2020-01-01T00:00:00Z')").bind(a.user.id).execute(&a.db).await.unwrap();
    let result =
        causal_group_lifecycle::import_operation(&a.db, &a.user, &a.identity, &peer, &rename).await;
    assert!(
        result.is_err(),
        "a reused numeric user id cannot inherit old lifecycle proof"
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models_group WHERE group_user_id=?")
        .bind(a.user.id)
        .fetch_one(&a.db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
#[allow(clippy::many_single_char_names)]
async fn expired_group_keep_preserves_move_made_after_purge() {
    let (a, b, c) = devices().await;
    let original = state(&a).await.entity;
    let h = group_db::add_empty_group(&a.db, &a.user, "destination", None)
        .await
        .unwrap();
    let genesis = causal_group_lifecycle::enroll_pristine_group(&a.db, &a.user, &a.identity, h.id)
        .await
        .unwrap();
    let ba = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    let ca = sync_peer::bind_peer(&c.db, &c.user, &c.identity, &a.identity)
        .await
        .unwrap();
    let bh =
        causal_genesis::install_pristine_genesis(&b.db, &b.user, &b.identity, &ba, &genesis, None)
            .await
            .unwrap()
            .local_id;
    let ch =
        causal_genesis::install_pristine_genesis(&c.db, &c.user, &c.identity, &ca, &genesis, None)
            .await
            .unwrap()
            .local_id;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.group), vec![a.model], None)
        .await
        .unwrap();
    let initial =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    transfer(&b, &a, &initial).await;
    transfer(&c, &a, &initial).await;
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    let deletion =
        causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &latest(&a).await)
            .await
            .unwrap();
    group_db::edit_group(&b.db, &b.user, b.group, "offline rename", None, None)
        .await
        .unwrap();
    let renamed =
        causal_group_lifecycle::export_operation(&b.db, &b.user, &b.identity, &latest(&b).await)
            .await
            .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(group_db::purge_trashed_groups(&a.db, 0).await.unwrap(), 1);
    group_db::set_group_id_on_models(&a.db, &a.user, Some(h.id), vec![a.model], None)
        .await
        .unwrap();
    let destination_state = causal_group_lifecycle::get_state(&a.db, &a.user, &a.identity, h.id)
        .await
        .unwrap();
    let dot = match destination_state.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected destination frontier {x:?}"),
    };
    let moved = causal_group_lifecycle::export_operation(&a.db, &a.user, &a.identity, &dot)
        .await
        .unwrap();
    transfer(&b, &a, &deletion).await;
    transfer(&b, &a, &moved).await;
    transfer(&c, &a, &deletion).await;
    transfer(&c, &a, &moved).await;
    transfer(&c, &b, &renamed).await;
    let peer = sync_peer::bind_peer(&a.db, &a.user, &a.identity, &b.identity)
        .await
        .unwrap();
    let result =
        causal_group_lifecycle::import_operation(&a.db, &a.user, &a.identity, &peer, &renamed)
            .await;
    assert!(
        matches!(result, Ok(OperationImportResult::Applied(_))),
        "offline keep must apply after purge: {result:?}"
    );
    for (d, target) in [(&a, h.id), (&b, bh), (&c, ch)] {
        let current: Option<i64> =
            sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
                .bind(d.model)
                .fetch_one(&d.db)
                .await
                .unwrap();
        assert_eq!(
            current,
            Some(target),
            "rehydration must not fabricate recovery or undo the independent move"
        );
    }
    let present:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models_group WHERE group_user_id=? AND group_unique_global_id=? AND group_birth_id=? AND group_deleted_at IS NULL)").bind(a.user.id).bind(&original.global_id).bind(&original.birth_id).fetch_one(&a.db).await.unwrap();
    assert!(present);
}

#[tokio::test]
async fn exact_restore_retry_survives_unrelated_scope_pause_but_not_later_delete() {
    let (a, _b, _c) = devices().await;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.group), vec![a.model], None)
        .await
        .unwrap();
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    let request = common::group_restore_request(&a.db, &a.user, a.group).await;
    group_db::restore_groups(&a.db, &a.user, std::slice::from_ref(&request), 30)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&a.db, &random_hex_32(), "stl", 3, None)
        .await
        .unwrap();
    let unrelated = model_db::add_model(&a.db, &a.user, "unrelated", blob, None, None)
        .await
        .unwrap();
    causal_group_lifecycle::enroll_pristine_model(&a.db, &a.user, &a.identity, unrelated)
        .await
        .unwrap();
    model_db::trash_models(&a.db, &a.user, &[unrelated])
        .await
        .unwrap();
    let before:(Option<i64>,i64,i64)=sqlx::query_as("SELECT model_group_id,(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM group_lifecycle_events) FROM models WHERE model_id=?").bind(a.model).fetch_one(&a.db).await.unwrap();
    assert_eq!(before.0, Some(a.group));
    let retry = group_db::restore_groups(&a.db, &a.user, std::slice::from_ref(&request), 30).await;
    assert!(
        retry.is_ok(),
        "exact restore retry must remain safe after unrelated pause: {retry:?}"
    );
    let after:(Option<i64>,i64,i64)=sqlx::query_as("SELECT model_group_id,(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM group_lifecycle_events) FROM models WHERE model_id=?").bind(a.model).fetch_one(&a.db).await.unwrap();
    assert_eq!(after, before);
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    assert!(
        group_db::restore_groups(&a.db, &a.user, &[request], 30)
            .await
            .is_err(),
        "old restore cannot cross a later deletion"
    );
}
