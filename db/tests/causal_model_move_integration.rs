//! Internal opaque move transfers do not authenticate or enable network sync.
use db::{
    blob_db, causal_genesis,
    causal_model_move::{self, MoveImportResult, MoveResolveResult, MoveState},
    group_db,
    model::user::User,
    model_db, random_hex_32, sync_peer, user_db,
};

mod common;

struct Device {
    dir: tempfile::TempDir,
    db: db::db_context::DbContext,
    user: User,
    identity: user_db::SyncIdentity,
    model: i64,
    groups: [i64; 3],
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
    let ba = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let ca = sync_peer::bind_peer(&c, &user, &ci, &ai).await.unwrap();
    let mut ag = Vec::new();
    let mut bg = Vec::new();
    let mut cg = Vec::new();
    for name in ["first", "second", "third"] {
        let group = group_db::add_empty_group(&a, &user, name, None)
            .await
            .unwrap();
        let bundle = causal_genesis::enroll_pristine_group(&a, &user, &ai, group.id)
            .await
            .unwrap();
        bg.push(
            causal_genesis::install_pristine_genesis(&b, &user, &bi, &ba, &bundle, None)
                .await
                .unwrap()
                .local_id,
        );
        cg.push(
            causal_genesis::install_pristine_genesis(&c, &user, &ci, &ca, &bundle, None)
                .await
                .unwrap()
                .local_id,
        );
        ag.push(group.id);
    }
    let hash = random_hex_32();
    let blob_a = blob_db::add_blob(&a, &hash, "stl", 10, None).await.unwrap();
    let blob_b = blob_db::add_blob(&b, &hash, "stl", 10, None).await.unwrap();
    let blob_c = blob_db::add_blob(&c, &hash, "stl", 10, None).await.unwrap();
    let model = model_db::add_model(&a, &user, "part", blob_a, None, None)
        .await
        .unwrap();
    let bundle = causal_genesis::enroll_pristine_model(&a, &user, &ai, model)
        .await
        .unwrap();
    let bm = causal_genesis::install_pristine_genesis(&b, &user, &bi, &ba, &bundle, Some(blob_b))
        .await
        .unwrap()
        .local_id;
    let cm = causal_genesis::install_pristine_genesis(&c, &user, &ci, &ca, &bundle, Some(blob_c))
        .await
        .unwrap()
        .local_id;
    (
        Device {
            dir: ad,
            db: a,
            user: user.clone(),
            identity: ai,
            model,
            groups: ag.try_into().unwrap(),
        },
        Device {
            dir: bd,
            db: b,
            user: user.clone(),
            identity: bi,
            model: bm,
            groups: bg.try_into().unwrap(),
        },
        Device {
            dir: cd,
            db: c,
            user,
            identity: ci,
            model: cm,
            groups: cg.try_into().unwrap(),
        },
    )
}
async fn state(device: &Device) -> MoveState {
    let result =
        causal_model_move::get_state(&device.db, &device.user, &device.identity, device.model)
            .await;
    assert!(
        result.is_ok(),
        "verified model genesis must expose move state: {result:?}"
    );
    result.unwrap()
}
async fn applied(
    device: &Device,
    source: &Device,
    event: &causal_model_move::MoveBundle,
) -> MoveState {
    let peer = sync_peer::bind_peer(&device.db, &device.user, &device.identity, &source.identity)
        .await
        .unwrap();
    match causal_model_move::import_event(&device.db, &device.user, &device.identity, &peer, event)
        .await
        .unwrap()
    {
        MoveImportResult::Applied(state) => state,
        x @ MoveImportResult::NeedsDependencies { .. } => panic!("expected applied event: {x:?}"),
    }
}
fn tip(move_state: MoveState) -> db::causal_envelope::Dot {
    match move_state {
        MoveState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready: {x:?}"),
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn three_devices_retain_conflicting_destinations_and_reject_stale_choice() {
    let (a, b, c) = three_devices().await;
    assert!(matches!(
        state(&a).await,
        MoveState::Ready {
            destination: None,
            ..
        }
    ));
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.groups[0]), vec![a.model], None)
        .await
        .unwrap();
    let initial = tip(state(&a).await);
    let initial_event = causal_model_move::export_event(&a.db, &a.user, &a.identity, &initial)
        .await
        .unwrap();
    applied(&b, &a, &initial_event).await;
    applied(&c, &a, &initial_event).await;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.groups[1]), vec![a.model], None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&b.db, &b.user, None, vec![b.model], None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&c.db, &c.user, Some(c.groups[2]), vec![c.model], None)
        .await
        .unwrap();
    let bd = tip(state(&b).await);
    let cd = tip(state(&c).await);
    let be = causal_model_move::export_event(&b.db, &b.user, &b.identity, &bd)
        .await
        .unwrap();
    let ce = causal_model_move::export_event(&c.db, &c.user, &c.identity, &cd)
        .await
        .unwrap();
    let (old_token, none_offered) = match applied(&a, &b, &be).await {
        MoveState::Conflict { token, choices } => (
            token,
            choices.iter().any(|choice| choice.destination.is_none()),
        ),
        x => panic!("expected conflict {x:?}"),
    };
    assert!(none_offered);
    let before:(Option<i64>,i64,i64)=sqlx::query_as("SELECT model_group_id,(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM causal_events) FROM models WHERE model_id=?").bind(a.model).fetch_one(&a.db).await.unwrap();
    assert!(
        group_db::set_group_id_on_models(&a.db, &a.user, Some(a.groups[1]), vec![a.model], None)
            .await
            .is_err(),
        "same-target move must not silently choose a conflicting branch"
    );
    let after:(Option<i64>,i64,i64)=sqlx::query_as("SELECT model_group_id,(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM causal_events) FROM models WHERE model_id=?").bind(a.model).fetch_one(&a.db).await.unwrap();
    assert_eq!(after, before);
    let token = match applied(&a, &c, &ce).await {
        MoveState::Conflict { token, choices } => {
            assert_eq!(choices.len(), 3);
            token
        }
        x => panic!("expected three branches {x:?}"),
    };
    assert!(matches!(
        causal_model_move::resolve(&a.db, &a.user, &a.identity, a.model, &old_token, None)
            .await
            .unwrap(),
        MoveResolveResult::Stale(_)
    ));
    let result = causal_model_move::resolve(&a.db, &a.user, &a.identity, a.model, &token, None)
        .await
        .unwrap();
    let resolved = match result {
        MoveResolveResult::Resolved(MoveState::Ready {
            destination: None,
            token,
        }) => token,
        x => panic!("expected resolved ungrouped {x:?}"),
    };
    let group: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(a.model)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert_eq!(group, None);
    let event = causal_model_move::export_event(&a.db, &a.user, &a.identity, &resolved.frontier[0])
        .await
        .unwrap();
    let mut parents = event.envelope().parents.clone();
    let mut expected = token.frontier.clone();
    parents.sort_by(|left, right| {
        (&left.writer_id, &left.counter).cmp(&(&right.writer_id, &right.counter))
    });
    expected.sort_by(|left, right| {
        (&left.writer_id, &left.counter).cmp(&(&right.writer_id, &right.counter))
    });
    assert_eq!(parents, expected);
}

#[tokio::test]
async fn missing_move_parent_requests_dependency_without_receipt_then_retry_after_restart() {
    let (a, mut b, c) = three_devices().await;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.groups[0]), vec![a.model], None)
        .await
        .unwrap();
    let parent = tip(state(&a).await);
    let parent_bundle = causal_model_move::export_event(&a.db, &a.user, &a.identity, &parent)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.groups[1]), vec![a.model], None)
        .await
        .unwrap();
    let child = tip(state(&a).await);
    let child_bundle = causal_model_move::export_event(&a.db, &a.user, &a.identity, &child)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    let before = state(&b).await;
    let result = causal_model_move::import_event(&b.db, &b.user, &b.identity, &peer, &child_bundle)
        .await
        .unwrap();
    assert!(
        matches!(result,MoveImportResult::NeedsDependencies{ref missing,ref state} if missing.iter().any(|dependency|dependency.event==parent) && *state==before)
    );
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM causal_events WHERE writer_id=? AND counter=?)",
    )
    .bind(&child.writer_id)
    .bind(child.counter.parse::<i64>().unwrap())
    .fetch_one(&b.db)
    .await
    .unwrap();
    assert!(
        !exists,
        "missing dependency response must not acknowledge or persist the child"
    );
    b.db.close().await;
    b.db = db::db_context::setup_db(
        &b.dir.path().join("db.sqlite"),
        &b.dir.path().join("backups"),
    )
    .await;
    assert_eq!(state(&b).await, before);
    applied(&b, &a, &parent_bundle).await;
    let ready = applied(&b, &a, &child_bundle).await;
    assert!(matches!(ready, MoveState::Ready { .. }));
    let relay = causal_model_move::export_event(&b.db, &b.user, &b.identity, &parent)
        .await
        .unwrap();
    applied(&c, &b, &relay).await;
    let relayed = causal_model_move::export_event(&c.db, &c.user, &c.identity, &parent)
        .await
        .unwrap();
    let original = state(&a).await;
    assert_eq!(
        applied(&a, &c, &relayed).await,
        original,
        "relaying original event back is an exact ACK"
    );
}

#[tokio::test]
async fn move_event_insertion_failure_rolls_back_assignment_and_causal_counter() {
    let (a, _b, _c) = three_devices().await;
    let before = state(&a).await;
    let counter: i64 =
        sqlx::query_scalar("SELECT counter FROM group_causal_replica WHERE singleton=1")
            .fetch_one(&a.db)
            .await
            .unwrap();
    sqlx::query("CREATE TRIGGER reject_move_event BEFORE INSERT ON model_move_events BEGIN SELECT RAISE(ABORT,'injected move failure'); END").execute(&a.db).await.unwrap();
    let error =
        group_db::set_group_id_on_models(&a.db, &a.user, Some(a.groups[0]), vec![a.model], None)
            .await
            .unwrap_err();
    assert!(error.to_string().contains("injected move failure"));
    let actual:(Option<i64>,i64)=sqlx::query_as("SELECT model_group_id,(SELECT counter FROM group_causal_replica WHERE singleton=1) FROM models WHERE model_id=?").bind(a.model).fetch_one(&a.db).await.unwrap();
    assert_eq!(actual, (None, counter));
    assert_eq!(state(&a).await, before);
}

#[tokio::test]
async fn equal_concurrent_destinations_keep_both_tips_and_next_move_observes_both() {
    let (a, b, _c) = three_devices().await;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.groups[0]), vec![a.model], None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&b.db, &b.user, Some(b.groups[0]), vec![b.model], None)
        .await
        .unwrap();
    let dot = tip(state(&b).await);
    let event = causal_model_move::export_event(&b.db, &b.user, &b.identity, &dot)
        .await
        .unwrap();
    let tips = match applied(&a, &b, &event).await {
        MoveState::Ready {
            destination: Some(_),
            token,
        } => {
            assert_eq!(token.frontier.len(), 2);
            token.frontier
        }
        x => panic!("expected equal ready tips: {x:?}"),
    };
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.groups[1]), vec![a.model], None)
        .await
        .unwrap();
    let next = tip(state(&a).await);
    let event = causal_model_move::export_event(&a.db, &a.user, &a.identity, &next)
        .await
        .unwrap();
    let mut actual = event.envelope().parents.clone();
    let mut expected = tips;
    actual.sort_by(|left, right| {
        (&left.writer_id, &left.counter).cmp(&(&right.writer_id, &right.counter))
    });
    expected.sort_by(|left, right| {
        (&left.writer_id, &left.counter).cmp(&(&right.writer_id, &right.counter))
    });
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn deleting_unselected_conflict_destination_pauses_and_restore_does_not_reactivate() {
    let (mut a, b, _c) = three_devices().await;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.groups[0]), vec![a.model], None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&b.db, &b.user, Some(b.groups[1]), vec![b.model], None)
        .await
        .unwrap();
    let dot = tip(state(&b).await);
    let event = causal_model_move::export_event(&b.db, &b.user, &b.identity, &dot)
        .await
        .unwrap();
    let (choices, token) = match applied(&a, &b, &event).await {
        MoveState::Conflict { choices, token } => (choices, token),
        x => panic!("expected conflict: {x:?}"),
    };
    let before: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(a.model)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert_eq!(before, Some(a.groups[0]));
    group_db::delete_group(&a.db, &a.user, a.groups[1])
        .await
        .unwrap();
    let request = common::group_restore_request(&a.db, &a.user, a.groups[1]).await;
    assert!(matches!(state(&a).await,MoveState::Paused{choices:ref actual,..} if *actual==choices));
    let result =
        causal_model_move::resolve(&a.db, &a.user, &a.identity, a.model, &token, None).await;
    assert!(result.is_err() || matches!(result, Ok(MoveResolveResult::Stale(_))));
    group_db::restore_groups(&a.db, &a.user, &[request], 30)
        .await
        .unwrap();
    a.db.close().await;
    a.db = db::db_context::setup_db(
        &a.dir.path().join("db.sqlite"),
        &a.dir.path().join("backups"),
    )
    .await;
    assert!(matches!(state(&a).await,MoveState::Paused{choices:ref actual,..} if *actual==choices));
    let after: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(a.model)
            .fetch_one(&a.db)
            .await
            .unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn second_model_move_failure_rolls_back_whole_batch() {
    let (a, _b, _c) = three_devices().await;
    let blob = blob_db::add_blob(&a.db, &random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let other = model_db::add_model(&a.db, &a.user, "other", blob, None, None)
        .await
        .unwrap();
    causal_genesis::enroll_pristine_model(&a.db, &a.user, &a.identity, other)
        .await
        .unwrap();
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM model_move_events")
        .fetch_one(&a.db)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER reject_second_move BEFORE INSERT ON model_move_events WHEN (SELECT COUNT(*) FROM model_move_events)=1 BEGIN SELECT RAISE(ABORT,'injected second move'); END").execute(&a.db).await.unwrap();
    let error = group_db::set_group_id_on_models(
        &a.db,
        &a.user,
        Some(a.groups[0]),
        vec![a.model, other],
        None,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("injected second move"));
    let groups: Vec<Option<i64>> = sqlx::query_scalar(
        "SELECT model_group_id FROM models WHERE model_id IN (?,?) ORDER BY model_id",
    )
    .bind(a.model)
    .bind(other)
    .fetch_all(&a.db)
    .await
    .unwrap();
    assert_eq!(groups, vec![None, None]);
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM model_move_events")
        .fetch_one(&a.db)
        .await
        .unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn resolution_rejects_destinations_not_offered_by_conflict() {
    let (a, b, _c) = three_devices().await;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.groups[0]), vec![a.model], None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&b.db, &b.user, Some(b.groups[1]), vec![b.model], None)
        .await
        .unwrap();
    let dot = tip(state(&b).await);
    let event = causal_model_move::export_event(&b.db, &b.user, &b.identity, &dot)
        .await
        .unwrap();
    let (token, choices) = match applied(&a, &b, &event).await {
        MoveState::Conflict { token, choices } => (token, choices),
        x => panic!("expected conflict: {x:?}"),
    };
    assert!(choices.iter().all(|choice| choice.destination.is_some()));
    let unrelated = causal_genesis::enroll_pristine_group(&a.db, &a.user, &a.identity, a.groups[2])
        .await
        .unwrap()
        .entity()
        .clone();
    let before:(Option<i64>,i64,i64)=sqlx::query_as("SELECT model_group_id,(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM causal_events) FROM models WHERE model_id=?").bind(a.model).fetch_one(&a.db).await.unwrap();
    for choice in [None, Some(unrelated)] {
        let result =
            causal_model_move::resolve(&a.db, &a.user, &a.identity, a.model, &token, choice).await;
        let error =
            result.expect_err("a destination absent from the presented conflict must be rejected");
        assert!(
            error
                .to_string()
                .contains("Selected destination is not an offered choice")
        );
        let after:(Option<i64>,i64,i64)=sqlx::query_as("SELECT model_group_id,(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM causal_events) FROM models WHERE model_id=?").bind(a.model).fetch_one(&a.db).await.unwrap();
        assert_eq!(after, before);
        assert_eq!(
            state(&a).await,
            MoveState::Conflict {
                token: token.clone(),
                choices: choices.clone()
            }
        );
    }
    let offered = choices[0].destination.clone();
    assert!(matches!(
        causal_model_move::resolve(&a.db, &a.user, &a.identity, a.model, &token, offered)
            .await
            .unwrap(),
        MoveResolveResult::Resolved(_)
    ));
}

#[tokio::test]
async fn relayed_resolution_cannot_introduce_unoffered_destination() {
    let (a, b, c) = three_devices().await;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.groups[0]), vec![a.model], None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&b.db, &b.user, Some(b.groups[1]), vec![b.model], None)
        .await
        .unwrap();
    let ad = tip(state(&a).await);
    let bd = tip(state(&b).await);
    let ae = causal_model_move::export_event(&a.db, &a.user, &a.identity, &ad)
        .await
        .unwrap();
    let be = causal_model_move::export_event(&b.db, &b.user, &b.identity, &bd)
        .await
        .unwrap();
    let token = match applied(&a, &b, &be).await {
        MoveState::Conflict { token, .. } => token,
        x => panic!("expected conflict {x:?}"),
    };
    applied(&c, &a, &ae).await;
    let before = applied(&c, &b, &be).await;
    assert!(matches!(before, MoveState::Conflict { .. }));
    let writer = random_hex_32();
    let forged = db::causal_envelope::CausalEnvelope {
        version: 2,
        entity: token.entity.clone(),
        event: db::causal_envelope::Dot {
            writer_id: writer.clone(),
            counter: "1".into(),
        },
        parents: token.frontier.clone(),
        dependencies: vec![],
        action: db::causal_envelope::Action::ModelDestinationResolved {
            to: None,
            target_lifecycle: None,
        },
    };
    let bytes = db::causal_envelope::canonical_causal_envelope(&forged).unwrap();
    sqlx::query("INSERT INTO causal_events(user_id,writer_id,counter,envelope) VALUES (?,?,1,?)")
        .bind(a.user.id)
        .bind(&writer)
        .bind(bytes)
        .execute(&a.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO model_move_events(user_id,birth_id,writer_id,counter,provenance) VALUES (?,?,?,1,'local')").bind(a.user.id).bind(token.entity.birth_id.as_ref().unwrap()).bind(&writer).execute(&a.db).await.unwrap();
    // The origin device already refuses to export an unoffered destination, and
    // MoveBundle cannot be built outside the crate, so export rejection is the first
    // defence. If export ever accepts the forged event, import must still reject it.
    match causal_model_move::export_event(&a.db, &a.user, &a.identity, &forged.event).await {
        Err(error) => assert!(
            matches!(error, db::DbError::Protocol(_)),
            "export must reject the forged resolution as a protocol error, got {error:?}"
        ),
        Ok(bundle) => {
            let peer = sync_peer::bind_peer(&c.db, &c.user, &c.identity, &a.identity)
                .await
                .unwrap();
            let received =
                causal_model_move::import_event(&c.db, &c.user, &c.identity, &peer, &bundle).await;
            assert!(
                received.is_err(),
                "a relayed resolution cannot invent an unoffered destination"
            );
        }
    }
    assert_eq!(state(&c).await, before);
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM causal_events WHERE writer_id=? AND counter=1)",
    )
    .bind(&writer)
    .fetch_one(&c.db)
    .await
    .unwrap();
    assert!(!present);
}

#[tokio::test]
async fn deleting_visible_preconflict_destination_retains_branch_choices() {
    let (a, b, c) = three_devices().await;
    group_db::set_group_id_on_models(&b.db, &b.user, Some(b.groups[0]), vec![b.model], None)
        .await
        .unwrap();
    let root = tip(state(&b).await);
    let root_event = causal_model_move::export_event(&b.db, &b.user, &b.identity, &root)
        .await
        .unwrap();
    applied(&c, &b, &root_event).await;
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.groups[1]), vec![a.model], None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&c.db, &c.user, Some(c.groups[2]), vec![c.model], None)
        .await
        .unwrap();
    let ad = tip(state(&a).await);
    let cd = tip(state(&c).await);
    let ae = causal_model_move::export_event(&a.db, &a.user, &a.identity, &ad)
        .await
        .unwrap();
    let ce = causal_model_move::export_event(&c.db, &c.user, &c.identity, &cd)
        .await
        .unwrap();
    applied(&b, &a, &ae).await;
    let choices = match applied(&b, &c, &ce).await {
        MoveState::Conflict { choices, .. } => choices,
        x => panic!("expected conflict {x:?}"),
    };
    let current: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(b.model)
            .fetch_one(&b.db)
            .await
            .unwrap();
    assert_eq!(current, Some(b.groups[0]));
    // C's move observes B's visible destination, removing it from the frontier;
    // the concurrent conflict keeps the preconflict destination visible.
    group_db::delete_group(&b.db, &b.user, b.groups[0])
        .await
        .unwrap();
    assert!(
        matches!(state(&b).await,MoveState::Paused{choices:ref retained,..} if *retained==choices),
        "deletion must not hide the unresolved alternatives"
    );
}

#[tokio::test]
async fn model_trash_and_restore_do_not_reactivate_old_move_scope_or_expose_it_to_other_owner() {
    let (a, _b, _c) = three_devices().await;
    let old = match state(&a).await {
        MoveState::Ready { token, .. } => token,
        x => panic!("expected ready: {x:?}"),
    };
    let other_id = user_db::add_user(&a.db, "other", "move-other@example.test", &random_hex_32())
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
    let foreign = causal_model_move::get_state(&a.db, &other, &other_identity, a.model).await;
    assert!(foreign.is_err() || matches!(foreign, Ok(MoveState::Unavailable)));
    model_db::trash_models(&a.db, &a.user, &[a.model])
        .await
        .unwrap();
    assert!(matches!(state(&a).await, MoveState::Unavailable));
    model_db::restore_models(&a.db, &a.user, &[a.model])
        .await
        .unwrap();
    assert!(matches!(state(&a).await, MoveState::Unavailable));
    let result = causal_model_move::resolve(&a.db, &a.user, &a.identity, a.model, &old, None).await;
    assert!(result.is_err() || matches!(result, Ok(MoveResolveResult::Stale(_))));
}

#[tokio::test]
async fn unknown_target_genesis_requests_dependency_without_ack_then_install_and_retry() {
    let (a, b, _c) = three_devices().await;
    let group = group_db::add_empty_group(&a.db, &a.user, "unshared", None)
        .await
        .unwrap();
    let genesis = causal_genesis::enroll_pristine_group(&a.db, &a.user, &a.identity, group.id)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a.db, &a.user, Some(group.id), vec![a.model], None)
        .await
        .unwrap();
    let dot = tip(state(&a).await);
    let event = causal_model_move::export_event(&a.db, &a.user, &a.identity, &dot)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&b.db, &b.user, &b.identity, &a.identity)
        .await
        .unwrap();
    let before = state(&b).await;
    let result = causal_model_move::import_event(&b.db, &b.user, &b.identity, &peer, &event).await;
    assert!(
        result.is_ok(),
        "missing target should request dependencies without admission: {result:?}"
    );
    let result = result.unwrap();
    assert!(
        matches!(result,MoveImportResult::NeedsDependencies{ref missing,ref state} if missing.iter().any(|dependency|dependency.event==*genesis.event() && dependency.entity==*genesis.entity()) && *state==before)
    );
    let stored: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM causal_events WHERE writer_id=? AND counter=?)",
    )
    .bind(&dot.writer_id)
    .bind(dot.counter.parse::<i64>().unwrap())
    .fetch_one(&b.db)
    .await
    .unwrap();
    assert!(!stored);
    let installed = causal_genesis::install_pristine_genesis(
        &b.db,
        &b.user,
        &b.identity,
        &peer,
        &genesis,
        None,
    )
    .await
    .unwrap();
    assert!(matches!(
        applied(&b, &a, &event).await,
        MoveState::Ready { .. }
    ));
    let target: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(b.model)
            .fetch_one(&b.db)
            .await
            .unwrap();
    assert_eq!(target, Some(installed.local_id));
}

#[tokio::test]
async fn blob_descriptor_aba_permanently_invalidates_move_scope() {
    let (a, _b, _c) = three_devices().await;
    let blob: i64 = sqlx::query_scalar("SELECT model_blob_id FROM models WHERE model_id=?")
        .bind(a.model)
        .fetch_one(&a.db)
        .await
        .unwrap();
    sqlx::query("UPDATE blobs SET blob_size=blob_size+1 WHERE blob_id=?")
        .bind(blob)
        .execute(&a.db)
        .await
        .unwrap();
    sqlx::query("UPDATE blobs SET blob_size=blob_size-1 WHERE blob_id=?")
        .bind(blob)
        .execute(&a.db)
        .await
        .unwrap();
    assert!(matches!(state(&a).await, MoveState::Unavailable));
    group_db::set_group_id_on_models(&a.db, &a.user, Some(a.groups[0]), vec![a.model], None)
        .await
        .unwrap();
    assert!(matches!(state(&a).await, MoveState::Unavailable));
}
