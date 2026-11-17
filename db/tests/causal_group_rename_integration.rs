//! Internal opaque event relay only; this does not authenticate or enable sync.
mod common;
use db::{
    causal_genesis,
    causal_group_rename::{self, NameState, ResolveResult},
    db_context::DbContext,
    group_db,
    model::user::User,
    sync_peer, user_db,
    user_db::SyncIdentity,
};

struct Device {
    dir: tempfile::TempDir,
    db: DbContext,
    user: User,
    identity: SyncIdentity,
    group: i64,
}

async fn three_devices() -> (Device, Device, Device) {
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
    let group = group_db::add_empty_group(&a, &user, "original", None)
        .await
        .unwrap();
    let genesis = causal_genesis::enroll_pristine_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let pb = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let pc = sync_peer::bind_peer(&c, &user, &ci, &ai).await.unwrap();
    let bg = causal_genesis::install_pristine_genesis(&b, &user, &bi, &pb, &genesis, None)
        .await
        .unwrap();
    let cg = causal_genesis::install_pristine_genesis(&c, &user, &ci, &pc, &genesis, None)
        .await
        .unwrap();
    (
        Device {
            dir: ad,
            db: a,
            user: user.clone(),
            identity: ai,
            group: group.id,
        },
        Device {
            dir: bd,
            db: b,
            user: user.clone(),
            identity: bi,
            group: bg.local_id,
        },
        Device {
            dir: cd,
            db: c,
            user,
            identity: ci,
            group: cg.local_id,
        },
    )
}

async fn state(device: &Device) -> NameState {
    let result =
        causal_group_rename::get_state(&device.db, &device.user, &device.identity, device.group)
            .await;
    assert!(
        result.is_ok(),
        "verified genesis must expose rename state: {result:?}"
    );
    result.unwrap()
}

async fn import_event(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    peer: &sync_peer::PeerBinding,
    event: &causal_group_rename::RenameBundle,
) -> Result<NameState, db::DbError> {
    match causal_group_rename::import_event(db, user, identity, peer, event).await? {
        causal_group_rename::RenameImportResult::Applied(state) => Ok(state),
        causal_group_rename::RenameImportResult::NeedsDependencies { .. } => {
            Err(db::DbError::Protocol("missing dependencies".into()))
        }
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn three_device_concurrent_renames_preserve_choices_and_stale_resolution() {
    let (a, b, c) = three_devices().await;
    assert!(
        matches!(state(&a).await, NameState::Ready { .. }),
        "enrolled genesis starts as a ready name"
    );
    group_db::edit_group(&a.db, &a.user, a.group, "alpha", None, None)
        .await
        .unwrap();
    group_db::edit_group(&b.db, &b.user, b.group, "bravo", None, None)
        .await
        .unwrap();
    group_db::edit_group(&c.db, &c.user, c.group, "charlie", None, None)
        .await
        .unwrap();
    let dot_b = match state(&b).await {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("unexpected state {x:?}"),
    };
    let dot_c = match state(&c).await {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("unexpected state {x:?}"),
    };
    let bb = causal_group_rename::export_event(&b.db, &b.user, &b.identity, &dot_b)
        .await
        .unwrap();
    let cb = causal_group_rename::export_event(&c.db, &c.user, &c.identity, &dot_c)
        .await
        .unwrap();
    let ab = sync_peer::bind_peer(&a.db, &a.user, &a.identity, &b.identity)
        .await
        .unwrap();
    let ac = sync_peer::bind_peer(&a.db, &a.user, &a.identity, &c.identity)
        .await
        .unwrap();
    let first = import_event(&a.db, &a.user, &a.identity, &ab, &bb)
        .await
        .unwrap();
    let (old_token, names) = match first {
        NameState::Conflict { token, choices } => (
            token,
            choices.into_iter().map(|x| x.name).collect::<Vec<_>>(),
        ),
        x => panic!("expected conflict, got {x:?}"),
    };
    assert!(names.contains(&"alpha".to_owned()) && names.contains(&"bravo".to_owned()));
    let before_unoffered = state(&a).await;
    let before_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events")
        .fetch_one(&a.db)
        .await
        .unwrap();
    assert!(
        causal_group_rename::resolve(
            &a.db,
            &a.user,
            &a.identity,
            a.group,
            &old_token,
            "unoffered"
        )
        .await
        .is_err(),
        "a choice must be one of the displayed names"
    );
    assert_eq!(state(&a).await, before_unoffered);
    let after_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events")
        .fetch_one(&a.db)
        .await
        .unwrap();
    assert_eq!(after_count, before_count);
    let visible: String =
        sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=?")
            .bind(a.group)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert_eq!(
        visible, "alpha",
        "a conflict must preserve the local visible name"
    );
    let before_edit: (String, i64, i64) = sqlx::query_as("SELECT group_name,(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM causal_events) FROM models_group WHERE group_id=?").bind(a.group).fetch_one(&a.db).await.unwrap();
    assert!(
        group_db::edit_group(&a.db, &a.user, a.group, "ordinary edit", None, None)
            .await
            .is_err()
    );
    let after_edit: (String, i64, i64) = sqlx::query_as("SELECT group_name,(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM causal_events) FROM models_group WHERE group_id=?").bind(a.group).fetch_one(&a.db).await.unwrap();
    assert_eq!(
        after_edit, before_edit,
        "ordinary edits cannot silently choose a conflict branch"
    );
    let retry = import_event(&a.db, &a.user, &a.identity, &ab, &bb)
        .await
        .unwrap();
    assert!(matches!(retry, NameState::Conflict { .. }));
    let current = import_event(&a.db, &a.user, &a.identity, &ac, &cb)
        .await
        .unwrap();
    let token = match current {
        NameState::Conflict { token, choices } => {
            assert_eq!(choices.len(), 3);
            token
        }
        x => panic!("expected three-way conflict, got {x:?}"),
    };
    assert!(matches!(
        causal_group_rename::resolve(&a.db, &a.user, &a.identity, a.group, &old_token, "alpha")
            .await
            .unwrap(),
        ResolveResult::Stale(_)
    ));
    assert_eq!(
        state(&a).await,
        NameState::Conflict {
            choices: match state(&a).await {
                NameState::Conflict { choices, .. } => choices,
                x => panic!("expected conflict: {x:?}"),
            },
            token: token.clone()
        }
    );
    let result =
        causal_group_rename::resolve(&a.db, &a.user, &a.identity, a.group, &token, "bravo")
            .await
            .unwrap();
    let resolved = match result {
        ResolveResult::Resolved(NameState::Ready { name, token }) => {
            assert_eq!(name, "bravo");
            token
        }
        x => panic!("expected resolution, got {x:?}"),
    };
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=?")
        .bind(a.group)
        .fetch_one(&a.db)
        .await
        .unwrap();
    assert_eq!(name, "bravo");
    assert_eq!(resolved.frontier.len(), 1);
    let envelope =
        causal_group_rename::export_event(&a.db, &a.user, &a.identity, &resolved.frontier[0])
            .await
            .unwrap();
    let mut parents = envelope.envelope().parents.clone();
    let mut expected = token.frontier.clone();
    parents.sort_by(|a, b| (&a.writer_id, &a.counter).cmp(&(&b.writer_id, &b.counter)));
    expected.sort_by(|a, b| (&a.writer_id, &a.counter).cmp(&(&b.writer_id, &b.counter)));
    assert_eq!(parents, expected);
}

#[tokio::test]
async fn failed_portable_event_write_rolls_back_name_and_frontier() {
    let (a, _b, _c) = three_devices().await;
    let before = state(&a).await;
    let before_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM causal_events WHERE user_id=?")
            .bind(a.user.id)
            .fetch_one(&a.db)
            .await
            .unwrap();
    sqlx::query("CREATE TRIGGER reject_name_event BEFORE INSERT ON group_rename_events BEGIN SELECT RAISE(ABORT,'injected rename failure'); END").execute(&a.db).await.unwrap();
    let error = group_db::edit_group(&a.db, &a.user, a.group, "after", None, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("injected rename failure"));
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=?")
        .bind(a.group)
        .fetch_one(&a.db)
        .await
        .unwrap();
    assert_eq!(name, "original");
    assert_eq!(state(&a).await, before);
    let after_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events WHERE user_id=?")
        .bind(a.user.id)
        .fetch_one(&a.db)
        .await
        .unwrap();
    assert_eq!(after_count, before_count);
}

#[tokio::test]
async fn unsupported_deletion_invalidates_scope_and_rejects_old_choice() {
    let (a, _b, _c) = three_devices().await;
    let token = match state(&a).await {
        NameState::Ready { token, .. } => token,
        x => panic!("expected ready: {x:?}"),
    };
    group_db::delete_group(&a.db, &a.user, a.group)
        .await
        .unwrap();
    assert!(matches!(state(&a).await, NameState::Unavailable));
    let result =
        causal_group_rename::resolve(&a.db, &a.user, &a.identity, a.group, &token, "original")
            .await;
    assert!(
        result.is_err() || matches!(result, Ok(ResolveResult::Stale(_))),
        "an unsupported deletion cannot revive an old choice"
    );
    let deleted: Option<String> =
        sqlx::query_scalar("SELECT group_deleted_at FROM models_group WHERE group_id=?")
            .bind(a.group)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert!(deleted.is_some());
}

#[tokio::test]
async fn concurrent_choices_survive_restart_and_remain_owner_scoped() {
    let (mut a, b, _c) = three_devices().await;
    group_db::edit_group(&a.db, &a.user, a.group, "alpha", None, None)
        .await
        .unwrap();
    group_db::edit_group(&b.db, &b.user, b.group, "bravo", None, None)
        .await
        .unwrap();
    let dot = match state(&b).await {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready: {x:?}"),
    };
    let event = causal_group_rename::export_event(&b.db, &b.user, &b.identity, &dot)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&a.db, &a.user, &a.identity, &b.identity)
        .await
        .unwrap();
    let expected = import_event(&a.db, &a.user, &a.identity, &peer, &event)
        .await
        .unwrap();
    assert!(matches!(expected, NameState::Conflict { .. }));
    a.db.close().await;
    a.db = db::db_context::setup_db(
        &a.dir.path().join("db.sqlite"),
        &a.dir.path().join("backups"),
    )
    .await;
    assert_eq!(state(&a).await, expected);
    let other_id = user_db::add_user(
        &a.db,
        "other",
        "rename-other@example.test",
        &db::random_hex_32(),
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
    let access = causal_group_rename::get_state(&a.db, &other, &other_identity, a.group).await;
    assert!(access.is_err() || matches!(access, Ok(NameState::Unavailable)));
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn child_before_parent_survives_restart_and_relay_back_acknowledges_original_dot() {
    let (a, mut b, c) = three_devices().await;
    group_db::edit_group(&a.db, &a.user, a.group, "first", None, None)
        .await
        .unwrap();
    let first = match state(&a).await {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready: {x:?}"),
    };
    group_db::edit_group(&a.db, &a.user, a.group, "second", None, None)
        .await
        .unwrap();
    let second = match state(&a).await {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready: {x:?}"),
    };
    group_db::edit_group(&a.db, &a.user, a.group, "third", None, None)
        .await
        .unwrap();
    let third = match state(&a).await {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready: {x:?}"),
    };
    let first_event = causal_group_rename::export_event(&a.db, &a.user, &a.identity, &first)
        .await
        .unwrap();
    let second_event = causal_group_rename::export_event(&a.db, &a.user, &a.identity, &second)
        .await
        .unwrap();
    let third_event = causal_group_rename::export_event(&a.db, &a.user, &a.identity, &third)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    let before = state(&b).await;
    let before_counts: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM group_rename_events WHERE user_id=?),(SELECT COUNT(*) FROM causal_event_receipts WHERE user_id=?),(SELECT COUNT(*) FROM causal_replica_owners WHERE user_id=?)").bind(b.user.id).bind(b.user.id).bind(b.user.id).fetch_one(&b.db).await.unwrap();
    let third_request =
        causal_group_rename::import_event(&b.db, &b.user, &b.identity, &peer, &third_event)
            .await
            .unwrap();
    assert!(
        matches!(third_request, causal_group_rename::RenameImportResult::NeedsDependencies { ref missing, ref state } if missing.contains(&second) && *state == before)
    );
    assert_eq!(
        causal_group_rename::export_event(&a.db, &a.user, &a.identity, &third)
            .await
            .unwrap()
            .envelope()
            .event,
        third,
        "the sender retains unacknowledged events"
    );
    let pending =
        causal_group_rename::import_event(&b.db, &b.user, &b.identity, &peer, &second_event)
            .await
            .unwrap();
    assert!(
        matches!(pending, causal_group_rename::RenameImportResult::NeedsDependencies { ref missing, ref state } if missing.contains(&first) && *state == before)
    );
    let after_counts: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM group_rename_events WHERE user_id=?),(SELECT COUNT(*) FROM causal_event_receipts WHERE user_id=?),(SELECT COUNT(*) FROM causal_replica_owners WHERE user_id=?)").bind(b.user.id).bind(b.user.id).bind(b.user.id).fetch_one(&b.db).await.unwrap();
    assert_eq!(
        after_counts, before_counts,
        "dependency requests must not admit metadata, receipts or authors"
    );
    let stored: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM causal_events WHERE writer_id=? AND counter=?")
            .bind(&second.writer_id)
            .bind(second.counter.parse::<i64>().unwrap())
            .fetch_one(&b.db)
            .await
            .unwrap();
    assert_eq!(
        stored, 0,
        "dependency request must not acknowledge or persist the child"
    );
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=?")
        .bind(b.group)
        .fetch_one(&b.db)
        .await
        .unwrap();
    assert_eq!(name, "original");
    b.db.close().await;
    b.db = db::db_context::setup_db(
        &b.dir.path().join("db.sqlite"),
        &b.dir.path().join("backups"),
    )
    .await;
    assert_eq!(state(&b).await, before);
    import_event(&b.db, &b.user, &b.identity, &peer, &first_event)
        .await
        .unwrap();
    let ready = import_event(&b.db, &b.user, &b.identity, &peer, &second_event)
        .await
        .unwrap();
    assert!(matches!(ready, NameState::Ready { ref name, .. } if name == "second"));
    let ready = import_event(&b.db, &b.user, &b.identity, &peer, &third_event)
        .await
        .unwrap();
    assert!(matches!(ready, NameState::Ready { ref name, .. } if name == "third"));
    let relayed = causal_group_rename::export_event(&b.db, &b.user, &b.identity, &first)
        .await
        .unwrap();
    assert_eq!(relayed.envelope().event, first);
    let peer_c = sync_peer::bind_peer(&c.db, &c.user, &c.identity, &b.identity)
        .await
        .unwrap();
    import_event(&c.db, &c.user, &c.identity, &peer_c, &relayed)
        .await
        .unwrap();
    let relayed_again = causal_group_rename::export_event(&c.db, &c.user, &c.identity, &first)
        .await
        .unwrap();
    assert_eq!(relayed_again.envelope().event, first);
    let peer_back = sync_peer::bind_peer(&a.db, &a.user, &a.identity, &c.identity)
        .await
        .unwrap();
    let before = state(&a).await;
    let repeated = import_event(&a.db, &a.user, &a.identity, &peer_back, &relayed_again)
        .await
        .unwrap();
    assert_eq!(
        repeated, before,
        "relaying the author's own event must be an idempotent ACK"
    );
}

#[tokio::test]
async fn equal_concurrent_names_keep_both_tips_and_next_edit_observes_both() {
    let (a, b, _c) = three_devices().await;
    group_db::edit_group(&a.db, &a.user, a.group, "same", None, None)
        .await
        .unwrap();
    group_db::edit_group(&b.db, &b.user, b.group, "same", None, None)
        .await
        .unwrap();
    let dot = match state(&b).await {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready: {x:?}"),
    };
    let event = causal_group_rename::export_event(&b.db, &b.user, &b.identity, &dot)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&a.db, &a.user, &a.identity, &b.identity)
        .await
        .unwrap();
    let before = import_event(&a.db, &a.user, &a.identity, &peer, &event)
        .await
        .unwrap();
    let tips = match before {
        NameState::Ready { name, token } => {
            assert_eq!(name, "same");
            assert_eq!(token.frontier.len(), 2);
            token.frontier
        }
        x => panic!("equal branches should be ready: {x:?}"),
    };
    group_db::edit_group(&a.db, &a.user, a.group, "after", None, None)
        .await
        .unwrap();
    let next = match state(&a).await {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready: {x:?}"),
    };
    let event = causal_group_rename::export_event(&a.db, &a.user, &a.identity, &next)
        .await
        .unwrap();
    let mut actual = event.envelope().parents.clone();
    let mut expected = tips;
    actual.sort_by(|a, b| (&a.writer_id, &a.counter).cmp(&(&b.writer_id, &b.counter)));
    expected.sort_by(|a, b| (&a.writer_id, &a.counter).cmp(&(&b.writer_id, &b.counter)));
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn pending_remote_history_keeps_existing_local_name() {
    let (a, b, _c) = three_devices().await;
    group_db::edit_group(&a.db, &a.user, a.group, "first", None, None)
        .await
        .unwrap();
    group_db::edit_group(&a.db, &a.user, a.group, "second", None, None)
        .await
        .unwrap();
    group_db::edit_group(&b.db, &b.user, b.group, "local", None, None)
        .await
        .unwrap();
    let child = match state(&a).await {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready {x:?}"),
    };
    let event = causal_group_rename::export_event(&a.db, &a.user, &a.identity, &child)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    let received = causal_group_rename::import_event(&b.db, &b.user, &b.identity, &peer, &event)
        .await
        .unwrap();
    assert!(matches!(
        received,
        causal_group_rename::RenameImportResult::NeedsDependencies { .. }
    ));
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=?")
        .bind(b.group)
        .fetch_one(&b.db)
        .await
        .unwrap();
    assert_eq!(
        name, "local",
        "unresolved remote history must not overwrite the local visible name"
    );
}

async fn seed_rename_history(device: &Device, count: usize, concurrent: bool) {
    use db::causal_envelope::{self, Action, CausalEnvelope, Dot};
    let root = match state(device).await {
        NameState::Ready { token, .. } => token,
        x => panic!("expected ready: {x:?}"),
    };
    let writer = db::random_hex_32();
    let mut parent = root.frontier[0].clone();
    let mut tx = device.db.begin().await.unwrap();
    for n in 1..=count {
        let dot = Dot {
            writer_id: writer.clone(),
            counter: n.to_string(),
        };
        let event = CausalEnvelope {
            version: 2,
            entity: root.entity.clone(),
            event: dot.clone(),
            parents: vec![if concurrent {
                root.frontier[0].clone()
            } else {
                parent.clone()
            }],
            dependencies: vec![],
            action: Action::GroupRenamed {
                name: "original".into(),
            },
        };
        let wire = causal_envelope::canonical_causal_envelope(&event).unwrap();
        sqlx::query("INSERT INTO causal_events (user_id,writer_id,counter,envelope,encoding) VALUES (?,?,?,?, 'portable_v2')").bind(device.user.id).bind(&writer).bind(i64::try_from(n).unwrap()).bind(wire).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO group_rename_events(user_id,birth_id,writer_id,counter,provenance) VALUES (?,?,?,?,'local')").bind(device.user.id).bind(root.entity.birth_id.as_ref().unwrap()).bind(&writer).bind(i64::try_from(n).unwrap()).execute(&mut *tx).await.unwrap();
        parent = dot;
    }
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn local_edit_continues_when_portable_history_reaches_capacity() {
    let (mut a, b, _c) = three_devices().await;
    group_db::edit_group(&b.db, &b.user, b.group, "remote", None, None)
        .await
        .unwrap();
    let remote_dot = match state(&b).await {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready: {x:?}"),
    };
    let remote = causal_group_rename::export_event(&b.db, &b.user, &b.identity, &remote_dot)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&a.db, &a.user, &a.identity, &b.identity)
        .await
        .unwrap();
    seed_rename_history(&a, db::causal_store::MAX_OWNER_EVENTS, false).await;
    let before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM group_rename_events WHERE user_id=?")
            .bind(a.user.id)
            .fetch_one(&a.db)
            .await
            .unwrap();
    let edit =
        group_db::edit_group(&a.db, &a.user, a.group, "local after capacity", None, None).await;
    assert!(
        edit.is_ok(),
        "a full portable journal must not block ordinary local edits: {edit:?}"
    );
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=?")
        .bind(a.group)
        .fetch_one(&a.db)
        .await
        .unwrap();
    assert_eq!(name, "local after capacity");
    let (token, retained) = match state(&a).await {
        NameState::Paused {
            name,
            choices,
            token,
            ..
        } => {
            assert_eq!(name, "local after capacity");
            assert!(choices.iter().any(|choice| choice.name == "original"));
            (token, choices)
        }
        other => panic!("expected paused state retaining prior choices: {other:?}"),
    };
    assert!(
        import_event(&a.db, &a.user, &a.identity, &peer, &remote)
            .await
            .is_err()
    );
    let result =
        causal_group_rename::resolve(&a.db, &a.user, &a.identity, a.group, &token, "original")
            .await;
    assert!(result.is_err() || matches!(result, Ok(ResolveResult::Stale(_))));
    a.db.close().await;
    a.db = db::db_context::setup_db(
        &a.dir.path().join("db.sqlite"),
        &a.dir.path().join("backups"),
    )
    .await;
    assert!(
        matches!(state(&a).await, NameState::Paused{name,choices,..} if name == "local after capacity" && choices == retained)
    );
    group_db::edit_group(&a.db, &a.user, a.group, "second local edit", None, None)
        .await
        .unwrap();
    assert!(
        matches!(state(&a).await, NameState::Paused{name,choices,..} if name == "second local edit" && choices == retained)
    );
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM group_rename_events WHERE user_id=?")
        .bind(a.user.id)
        .fetch_one(&a.db)
        .await
        .unwrap();
    assert_eq!(
        after, before,
        "existing causal evidence must not be discarded to make room"
    );
    let local_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM causal_events WHERE user_id=? AND encoding='local_history_v1'",
    )
    .bind(a.user.id)
    .fetch_one(&a.db)
    .await
    .unwrap();
    assert!(local_events > 0, "the local-only edit must remain durable");
}

#[tokio::test]
async fn remote_event_past_resolvable_frontier_is_rejected_without_losing_branches() {
    let (a, b, _c) = three_devices().await;
    group_db::edit_group(&a.db, &a.user, a.group, "remote", None, None)
        .await
        .unwrap();
    let dot = match state(&a).await {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready {x:?}"),
    };
    let event = causal_group_rename::export_event(&a.db, &a.user, &a.identity, &dot)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    seed_rename_history(&b, db::causal_envelope::MAX_CONTEXT_ITEMS, true).await;
    let before = state(&b).await;
    assert!(
        matches!(&before, NameState::Ready{token,..} if token.frontier.len()==db::causal_envelope::MAX_CONTEXT_ITEMS)
    );
    let received = import_event(&b.db, &b.user, &b.identity, &peer, &event).await;
    assert!(
        received.is_err(),
        "a frontier too large to resolve must not be admitted"
    );
    assert_eq!(state(&b).await, before);
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM causal_events WHERE writer_id=? AND counter=?")
            .bind(&dot.writer_id)
            .bind(dot.counter.parse::<i64>().unwrap())
            .fetch_one(&b.db)
            .await
            .unwrap();
    assert_eq!(
        count, 0,
        "rejected admission must not persist a partial receipt"
    );
}

#[tokio::test]
async fn paused_scope_exposes_missing_ancestry_after_capacity_fallback() {
    use db::causal_envelope::{self, Action, CausalEnvelope, Dot};
    let (a, _b, _c) = three_devices().await;
    let base = match state(&a).await {
        NameState::Ready { token, .. } => token,
        x => panic!("expected ready {x:?}"),
    };
    let missing = Dot {
        writer_id: db::random_hex_32(),
        counter: "1".into(),
    };
    let writer = db::random_hex_32();
    let mut parent = missing.clone();
    let mut tx = a.db.begin().await.unwrap();
    for n in 1..=db::causal_store::MAX_OWNER_EVENTS {
        let dot = Dot {
            writer_id: writer.clone(),
            counter: n.to_string(),
        };
        let event = CausalEnvelope {
            version: 2,
            entity: base.entity.clone(),
            event: dot.clone(),
            parents: vec![parent],
            dependencies: vec![],
            action: Action::GroupRenamed {
                name: "unresolved".into(),
            },
        };
        sqlx::query("INSERT INTO causal_events (user_id,writer_id,counter,envelope,encoding) VALUES (?,?,?,?,'portable_v2')").bind(a.user.id).bind(&writer).bind(i64::try_from(n).unwrap()).bind(causal_envelope::canonical_causal_envelope(&event).unwrap()).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO group_rename_events(user_id,birth_id,writer_id,counter,provenance) VALUES (?,?,?,?,'checked_transfer')").bind(a.user.id).bind(base.entity.birth_id.as_ref().unwrap()).bind(&writer).bind(i64::try_from(n).unwrap()).execute(&mut *tx).await.unwrap();
        parent = dot;
    }
    tx.commit().await.unwrap();
    assert!(matches!(state(&a).await,NameState::Paused{ref missing,..} if missing.len()==1));
    group_db::edit_group(&a.db, &a.user, a.group, "local", None, None)
        .await
        .unwrap();
    match state(&a).await {
        NameState::Paused {
            name,
            missing: pending,
            ..
        } => {
            assert_eq!(name, "local");
            assert_eq!(pending, vec![missing]);
        }
        x => panic!("paused scope must expose unresolved ancestry: {x:?}"),
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn dependency_request_at_full_receipt_quotas_writes_nothing_and_is_not_ack() {
    use db::causal_envelope::{self, Action, CausalEnvelope, Dot};
    let (a, b, _c) = three_devices().await;
    group_db::edit_group(&a.db, &a.user, a.group, "parent", None, None)
        .await
        .unwrap();
    let parent = match state(&a).await {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready {x:?}"),
    };
    group_db::edit_group(&a.db, &a.user, a.group, "child", None, None)
        .await
        .unwrap();
    let child = match state(&a).await {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready {x:?}"),
    };
    let bundle = causal_group_rename::export_event(&a.db, &a.user, &a.identity, &child)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    let entity = match state(&b).await {
        NameState::Ready { token, .. } => token.entity,
        x => panic!("expected ready {x:?}"),
    };
    let writer = db::random_hex_32();
    sqlx::query("INSERT INTO causal_replica_owners(user_id,replica_id) VALUES (?,?)")
        .bind(b.user.id)
        .bind(&writer)
        .execute(&b.db)
        .await
        .unwrap();
    let mut tx = b.db.begin().await.unwrap();
    let large_count = db::causal_store::MAX_OWNER_BYTES / causal_envelope::MAX_NAME_BYTES + 1;
    for n in 1..db::causal_store::MAX_OWNER_EVENTS {
        let dot = Dot {
            writer_id: writer.clone(),
            counter: n.to_string(),
        };
        let edge_count = if n == db::causal_store::MAX_OWNER_EVENTS - 1 {
            16
        } else {
            8
        };
        let parents = (1..=edge_count)
            .map(|k| Dot {
                writer_id: db::random_hex_32(),
                counter: k.to_string(),
            })
            .collect::<Vec<_>>();
        let event = CausalEnvelope {
            version: 2,
            entity: entity.clone(),
            event: dot,
            parents: parents.clone(),
            dependencies: vec![],
            action: Action::GroupRenamed {
                name: if n <= large_count {
                    "x".repeat(causal_envelope::MAX_NAME_BYTES)
                } else {
                    "x".into()
                },
            },
        };
        sqlx::query(
            "INSERT INTO causal_events(user_id,writer_id,counter,envelope) VALUES (?,?,?,?)",
        )
        .bind(b.user.id)
        .bind(&writer)
        .bind(i64::try_from(n).unwrap())
        .bind(causal_envelope::canonical_causal_envelope(&event).unwrap())
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query("INSERT INTO causal_event_receipts(user_id,writer_id,counter) VALUES (?,?,?)")
            .bind(b.user.id)
            .bind(&writer)
            .bind(i64::try_from(n).unwrap())
            .execute(&mut *tx)
            .await
            .unwrap();
        for p in parents {
            sqlx::query("INSERT INTO causal_receipt_dependencies(user_id,writer_id,counter,dependency_writer_id,dependency_counter) VALUES (?,?,?,?,?)").bind(b.user.id).bind(&writer).bind(i64::try_from(n).unwrap()).bind(p.writer_id).bind(1_i64).execute(&mut *tx).await.unwrap();
        }
    }
    tx.commit().await.unwrap();
    let before:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_event_receipts WHERE user_id=?),(SELECT COALESCE(SUM(length(CAST(e.envelope AS BLOB))),0) FROM causal_event_receipts r JOIN causal_events e USING(user_id,writer_id,counter) WHERE r.user_id=?),(SELECT COUNT(*) FROM causal_receipt_dependencies WHERE user_id=?),(SELECT COUNT(*) FROM causal_replica_owners WHERE user_id=?)").bind(b.user.id).bind(b.user.id).bind(b.user.id).bind(b.user.id).fetch_one(&b.db).await.unwrap();
    assert_eq!(
        usize::try_from(before.0).unwrap(),
        db::causal_store::MAX_OWNER_EVENTS
    );
    assert!(usize::try_from(before.1).unwrap() >= db::causal_store::MAX_OWNER_BYTES);
    assert_eq!(
        usize::try_from(before.2).unwrap(),
        db::causal_store::MAX_OWNER_DEPENDENCIES
    );
    let prior = state(&b).await;
    let result = causal_group_rename::import_event(&b.db, &b.user, &b.identity, &peer, &bundle)
        .await
        .unwrap();
    assert!(
        matches!(result,causal_group_rename::RenameImportResult::NeedsDependencies{ref missing,ref state} if missing.contains(&parent) && *state==prior)
    );
    let after:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_event_receipts WHERE user_id=?),(SELECT COALESCE(SUM(length(CAST(e.envelope AS BLOB))),0) FROM causal_event_receipts r JOIN causal_events e USING(user_id,writer_id,counter) WHERE r.user_id=?),(SELECT COUNT(*) FROM causal_receipt_dependencies WHERE user_id=?),(SELECT COUNT(*) FROM causal_replica_owners WHERE user_id=?)").bind(b.user.id).bind(b.user.id).bind(b.user.id).bind(b.user.id).fetch_one(&b.db).await.unwrap();
    assert_eq!(after, before);
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM causal_events WHERE writer_id=? AND counter=?)",
    )
    .bind(&child.writer_id)
    .bind(child.counter.parse::<i64>().unwrap())
    .fetch_one(&b.db)
    .await
    .unwrap();
    assert!(!exists);
    assert_eq!(
        causal_group_rename::export_event(&a.db, &a.user, &a.identity, &child)
            .await
            .unwrap()
            .envelope()
            .event,
        child
    );
}
