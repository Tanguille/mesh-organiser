//! Received rename history stays owner-scoped and never mutates applied domain state.
mod common;

use db::{
    blob_db,
    db_context::DbContext,
    group_db,
    group_history::{self, EventId, GroupRenameEvent, IngestResult, ReplicaPrincipal},
    model::user::User,
    model_db, random_hex_32, user_db,
};

async fn trust(db: &DbContext, user: &User, replica: &str) -> ReplicaPrincipal {
    sqlx::query("INSERT INTO causal_replica_owners (user_id, replica_id) VALUES (?, ?)")
        .bind(user.id)
        .bind(replica)
        .execute(db)
        .await
        .unwrap();
    ReplicaPrincipal {
        owner_user_id: user.id,
        replica_id: replica.into(),
    }
}

fn make_event(
    replica: &str,
    counter: i64,
    group: &str,
    name: &str,
    parents: Vec<EventId>,
) -> GroupRenameEvent {
    GroupRenameEvent {
        id: EventId {
            replica_id: replica.into(),
            counter,
        },
        group_global_id: group.into(),
        parents,
        name: name.into(),
    }
}

async fn domain_state(
    db: &DbContext,
    user: &User,
    group_id: i64,
    global: &str,
) -> (String, i64, i64, i64, Vec<EventId>, usize) {
    let (name, revision): (String, i64) = sqlx::query_as("SELECT group_name, group_revision FROM models_group WHERE group_id = ? AND group_user_id = ?").bind(group_id).bind(user.id).fetch_one(db).await.unwrap();
    let head: i64 = sqlx::query_scalar(
        "SELECT head FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(global)
    .fetch_one(db)
    .await
    .unwrap();
    let counter: i64 =
        sqlx::query_scalar("SELECT counter FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(db)
            .await
            .unwrap();
    let history = group_history::get_group_history(db, user, global)
        .await
        .unwrap()
        .unwrap();
    (
        name,
        revision,
        head,
        counter,
        history.frontier,
        history.events.len(),
    )
}

#[tokio::test]
async fn rename_ingestion_is_exactly_replayable_and_never_changes_applied_state() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "local name", None)
        .await
        .unwrap();
    let replica = random_hex_32();
    let principal = trust(&db, &user, &replica).await;
    let event = make_event(&replica, 1, &group.unique_global_id, "remote name", vec![]);
    let before = domain_state(&db, &user, group.id, &group.unique_global_id).await;
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &event)
            .await
            .unwrap(),
        IngestResult::AcceptedReady
    ));
    assert_eq!(
        domain_state(&db, &user, group.id, &group.unique_global_id).await,
        before
    );
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &event)
            .await
            .unwrap(),
        IngestResult::AlreadyPresent
    ));
    assert_eq!(
        domain_state(&db, &user, group.id, &group.unique_global_id).await,
        before
    );
    let mut changed = event.clone();
    changed.name = "different".into();
    assert!(
        group_history::ingest_group_rename(&db, &principal, &changed)
            .await
            .is_err()
    );
    let received = group_history::get_received_group_renames(&db, &user, &group.unique_global_id)
        .await
        .unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].event, event);
    assert_eq!(received[0].missing, vec![]);
}

#[tokio::test]
async fn missing_parents_are_retained_across_restart_until_they_arrive() {
    let (dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "local", None)
        .await
        .unwrap();
    let replica = random_hex_32();
    let principal = trust(&db, &user, &replica).await;
    let parent = make_event(&replica, 1, &group.unique_global_id, "parent", vec![]);
    let child = make_event(
        &replica,
        2,
        &group.unique_global_id,
        "child",
        vec![parent.id.clone()],
    );
    let before = domain_state(&db, &user, group.id, &group.unique_global_id).await;
    assert!(
        matches!(group_history::ingest_group_rename(&db, &principal, &child).await.unwrap(), IngestResult::AcceptedPending { missing } if missing == vec![parent.id.clone()])
    );
    let old_writer: String =
        sqlx::query_scalar("SELECT replica_id FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    db.close().await;
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    let (new_writer, counter): (String, i64) =
        sqlx::query_as("SELECT replica_id, counter FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_ne!(new_writer, old_writer);
    assert_eq!(counter, 0);
    let mut after_restart = before;
    after_restart.3 = 0;
    let pending = group_history::get_received_group_renames(&db, &user, &group.unique_global_id)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].event, child);
    assert_eq!(pending[0].missing, vec![parent.id.clone()]);
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &parent)
            .await
            .unwrap(),
        IngestResult::AcceptedReady
    ));
    let received = group_history::get_received_group_renames(&db, &user, &group.unique_global_id)
        .await
        .unwrap();
    assert_eq!(received.len(), 2);
    assert!(received.iter().all(|entry| entry.missing.is_empty()));
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &child)
            .await
            .unwrap(),
        IngestResult::AlreadyPresent
    ));
    assert_eq!(
        domain_state(&db, &user, group.id, &group.unique_global_id).await,
        after_restart
    );
}

#[tokio::test]
async fn owner_registration_and_authenticated_replica_are_enforced() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &owner, "private", None)
        .await
        .unwrap();
    let replica = random_hex_32();
    let principal = trust(&db, &owner, &replica).await;
    let event = make_event(&replica, 1, &group.unique_global_id, "remote", vec![]);
    let unregistered = ReplicaPrincipal {
        owner_user_id: owner.id,
        replica_id: random_hex_32(),
    };
    let unregistered_event = make_event(
        &unregistered.replica_id,
        1,
        &group.unique_global_id,
        "remote",
        vec![],
    );
    assert!(
        group_history::ingest_group_rename(&db, &unregistered, &unregistered_event)
            .await
            .is_err()
    );
    assert!(
        group_history::ingest_group_rename(&db, &unregistered, &event)
            .await
            .is_err()
    );
    let foreign = trust(&db, &other, &random_hex_32()).await;
    let foreign_event = make_event(
        &foreign.replica_id,
        1,
        &group.unique_global_id,
        "attack",
        vec![],
    );
    assert!(
        group_history::ingest_group_rename(&db, &foreign, &foreign_event)
            .await
            .is_err()
    );
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &event)
            .await
            .unwrap(),
        IngestResult::AcceptedReady
    ));
    assert_eq!(
        group_history::get_received_group_renames(&db, &other, &group.unique_global_id)
            .await
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn membership_dot_cannot_be_reused_for_a_group_rename() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, "model", "stl", 1, None)
        .await
        .unwrap();
    let id = model_db::add_model(&db, &user, "model", blob, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![id], None)
        .await
        .unwrap();
    let (global, incarnation, birth, envelope): (String, String, Option<String>, String) = sqlx::query_as(
        "SELECT m.model_global_id,m.model_incarnation,m.model_birth_id,e.envelope FROM membership_causal_events m JOIN causal_events e ON e.user_id=m.user_id AND e.writer_id=m.replica_id AND e.counter=m.counter WHERE m.user_id=? LIMIT 1",
    ).bind(user.id).fetch_one(&db).await.unwrap();
    let counter = 1_i64;
    let replica = random_hex_32();
    sqlx::query("INSERT INTO causal_events (user_id,writer_id,counter,envelope,encoding) VALUES (?,?,?,?,'local_history_v1')")
        .bind(user.id).bind(&replica).bind(counter).bind(envelope).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO membership_causal_events (user_id,model_global_id,model_incarnation,replica_id,counter,model_birth_id) VALUES (?,?,?,?,?,?)")
        .bind(user.id).bind(global).bind(incarnation).bind(&replica).bind(counter).bind(birth).execute(&db).await.unwrap();
    let principal = trust(&db, &user, &replica).await;
    let before = domain_state(&db, &user, group.id, &group.unique_global_id).await;
    let duplicate = make_event(&replica, counter, &group.unique_global_id, "forged", vec![]);
    assert!(
        group_history::ingest_group_rename(&db, &principal, &duplicate)
            .await
            .is_err()
    );
    assert_eq!(
        domain_state(&db, &user, group.id, &group.unique_global_id).await,
        before
    );
    assert_eq!(
        group_history::get_received_group_renames(&db, &user, &group.unique_global_id)
            .await
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn parent_order_is_canonical_but_changed_dependencies_are_rejected() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    let replica = random_hex_32();
    let principal = trust(&db, &user, &replica).await;
    let first = make_event(&replica, 1, &group.unique_global_id, "first", vec![]);
    let second = make_event(&replica, 2, &group.unique_global_id, "second", vec![]);
    group_history::ingest_group_rename(&db, &principal, &first)
        .await
        .unwrap();
    group_history::ingest_group_rename(&db, &principal, &second)
        .await
        .unwrap();
    let child = make_event(
        &replica,
        3,
        &group.unique_global_id,
        "merged",
        vec![first.id.clone(), second.id.clone()],
    );
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &child)
            .await
            .unwrap(),
        IngestResult::AcceptedReady
    ));
    let mut reordered = child.clone();
    reordered.parents.reverse();
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &reordered)
            .await
            .unwrap(),
        IngestResult::AlreadyPresent
    ));
    let mut altered = child.clone();
    altered.parents.pop();
    assert!(
        group_history::ingest_group_rename(&db, &principal, &altered)
            .await
            .is_err()
    );
    let mut repeated = make_event(
        &replica,
        4,
        &group.unique_global_id,
        "bad",
        vec![first.id.clone(), first.id.clone()],
    );
    assert!(
        group_history::ingest_group_rename(&db, &principal, &repeated)
            .await
            .is_err()
    );
    repeated.parents = vec![repeated.id.clone()];
    assert!(
        group_history::ingest_group_rename(&db, &principal, &repeated)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn pending_events_do_not_allow_cycles_or_wrong_group_parents() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let a = group_db::add_empty_group(&db, &user, "A", None)
        .await
        .unwrap();
    let b = group_db::add_empty_group(&db, &user, "B", None)
        .await
        .unwrap();
    let replica = random_hex_32();
    let principal = trust(&db, &user, &replica).await;
    let other = make_event(&replica, 1, &b.unique_global_id, "other", vec![]);
    group_history::ingest_group_rename(&db, &principal, &other)
        .await
        .unwrap();
    let invalid = make_event(
        &replica,
        2,
        &a.unique_global_id,
        "invalid",
        vec![other.id.clone()],
    );
    assert!(
        group_history::ingest_group_rename(&db, &principal, &invalid)
            .await
            .is_err()
    );
    let second_replica = random_hex_32();
    let second_principal = trust(&db, &user, &second_replica).await;
    let later = EventId {
        replica_id: second_replica.clone(),
        counter: 4,
    };
    let earlier = make_event(&replica, 3, &a.unique_global_id, "earlier", vec![later]);
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &earlier)
            .await
            .unwrap(),
        IngestResult::AcceptedPending { .. }
    ));
    let cycle = make_event(
        &second_replica,
        4,
        &a.unique_global_id,
        "cycle",
        vec![earlier.id.clone()],
    );
    assert!(
        group_history::ingest_group_rename(&db, &second_principal, &cycle)
            .await
            .is_err()
    );
    let received = group_history::get_received_group_renames(&db, &user, &a.unique_global_id)
        .await
        .unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].event, earlier);
    assert_eq!(received[0].missing, vec![cycle.id]);
}

#[tokio::test]
async fn parent_owned_by_another_user_is_never_accepted_as_causal_history() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let foreign_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let foreign = user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &owner, "owner group", None)
        .await
        .unwrap();
    let other_group = group_db::add_empty_group(&db, &foreign, "other group", None)
        .await
        .unwrap();
    sqlx::query("UPDATE models_group SET group_unique_global_id = ? WHERE group_id = ?")
        .bind(&group.unique_global_id)
        .bind(other_group.id)
        .execute(&db)
        .await
        .unwrap();
    let own_replica = random_hex_32();
    let foreign_replica = random_hex_32();
    let own_principal = trust(&db, &owner, &own_replica).await;
    let foreign_principal = trust(&db, &foreign, &foreign_replica).await;
    let parent = make_event(
        &foreign_replica,
        1,
        &group.unique_global_id,
        "foreign",
        vec![],
    );
    group_history::ingest_group_rename(&db, &foreign_principal, &parent)
        .await
        .unwrap();
    let child = make_event(
        &own_replica,
        1,
        &group.unique_global_id,
        "owner",
        vec![parent.id],
    );
    assert!(
        group_history::ingest_group_rename(&db, &own_principal, &child)
            .await
            .is_err()
    );
    assert_eq!(
        group_history::get_received_group_renames(&db, &owner, &group.unique_global_id)
            .await
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn rename_ingestion_enforces_parent_count_and_utf8_name_byte_limits() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    let replica = random_hex_32();
    let principal = trust(&db, &user, &replica).await;
    assert_eq!(group_history::MAX_RENAME_EVENT_PARENTS, 1024);
    assert_eq!(group_history::MAX_RENAME_EVENT_NAME_BYTES, 65536);
    let parents: Vec<_> = (1..=group_history::MAX_RENAME_EVENT_PARENTS)
        .map(|counter| EventId {
            replica_id: replica.clone(),
            counter: i64::try_from(counter).unwrap(),
        })
        .collect();
    let boundary = make_event(
        &replica,
        2000,
        &group.unique_global_id,
        &"x".repeat(group_history::MAX_RENAME_EVENT_NAME_BYTES),
        parents.clone(),
    );
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &boundary)
            .await
            .unwrap(),
        IngestResult::AcceptedPending { .. }
    ));
    let mut too_many = parents;
    too_many.push(EventId {
        replica_id: replica.clone(),
        counter: 1500,
    });
    let event = make_event(&replica, 2001, &group.unique_global_id, "name", too_many);
    assert!(
        group_history::ingest_group_rename(&db, &principal, &event)
            .await
            .is_err()
    );
    let oversized = make_event(
        &replica,
        2002,
        &group.unique_global_id,
        &format!(
            "{}é",
            "x".repeat(group_history::MAX_RENAME_EVENT_NAME_BYTES - 1)
        ),
        vec![],
    );
    assert!(
        group_history::ingest_group_rename(&db, &principal, &oversized)
            .await
            .is_err()
    );
    let received = group_history::get_received_group_renames(&db, &user, &group.unique_global_id)
        .await
        .unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].event.id, boundary.id);
}

#[tokio::test]
async fn edge_insert_failure_rolls_back_the_event_reservation_and_allows_retry() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    let replica = random_hex_32();
    let principal = trust(&db, &user, &replica).await;
    let parent = EventId {
        replica_id: replica.clone(),
        counter: 1,
    };
    let child = make_event(
        &replica,
        2,
        &group.unique_global_id,
        "child",
        vec![parent.clone()],
    );
    let before = domain_state(&db, &user, group.id, &group.unique_global_id).await;
    sqlx::query("CREATE TRIGGER reject_parent_edge BEFORE INSERT ON received_group_rename_parents BEGIN SELECT RAISE(ABORT, 'injected edge failure'); END").execute(&db).await.unwrap();
    assert!(
        group_history::ingest_group_rename(&db, &principal, &child)
            .await
            .is_err()
    );
    assert_eq!(
        group_history::get_received_group_renames(&db, &user, &group.unique_global_id)
            .await
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        domain_state(&db, &user, group.id, &group.unique_global_id).await,
        before
    );
    sqlx::query("DROP TRIGGER reject_parent_edge")
        .execute(&db)
        .await
        .unwrap();
    assert!(
        matches!(group_history::ingest_group_rename(&db, &principal, &child).await.unwrap(), IngestResult::AcceptedPending { missing } if missing == vec![parent])
    );
}

#[tokio::test]
async fn local_replica_cannot_be_impersonated_by_incoming_future_dot() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    let (replica, counter): (String, i64) =
        sqlx::query_as("SELECT replica_id, counter FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    let principal = trust(&db, &user, &replica).await;
    let forged = make_event(
        &replica,
        counter + 1,
        &group.unique_global_id,
        "forged",
        vec![],
    );
    let before = domain_state(&db, &user, group.id, &group.unique_global_id).await;
    assert!(
        group_history::ingest_group_rename(&db, &principal, &forged)
            .await
            .is_err()
    );
    assert_eq!(
        domain_state(&db, &user, group.id, &group.unique_global_id).await,
        before
    );
}

#[tokio::test]
async fn transitive_dependencies_remain_pending_until_root_arrives() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    let replica = random_hex_32();
    let principal = trust(&db, &user, &replica).await;
    let root = make_event(&replica, 1, &group.unique_global_id, "root", vec![]);
    let middle = make_event(
        &replica,
        2,
        &group.unique_global_id,
        "middle",
        vec![root.id.clone()],
    );
    let leaf = make_event(
        &replica,
        3,
        &group.unique_global_id,
        "leaf",
        vec![middle.id.clone()],
    );
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &middle)
            .await
            .unwrap(),
        IngestResult::AcceptedPending { .. }
    ));
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &leaf)
            .await
            .unwrap(),
        IngestResult::AcceptedPending { .. }
    ));
    group_history::ingest_group_rename(&db, &principal, &root)
        .await
        .unwrap();
    let all = group_history::get_received_group_renames(&db, &user, &group.unique_global_id)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    assert!(all.iter().all(|event| event.missing.is_empty()));
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &leaf)
            .await
            .unwrap(),
        IngestResult::AlreadyPresent
    ));
}

#[tokio::test]
async fn wrong_group_pending_reference_cannot_block_a_valid_parent() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let a = group_db::add_empty_group(&db, &user, "A", None)
        .await
        .unwrap();
    let b = group_db::add_empty_group(&db, &user, "B", None)
        .await
        .unwrap();
    let replica = random_hex_32();
    let principal = trust(&db, &user, &replica).await;
    let parent = make_event(&replica, 1, &b.unique_global_id, "wrong group", vec![]);
    let child = make_event(
        &replica,
        2,
        &a.unique_global_id,
        "waiting",
        vec![parent.id.clone()],
    );
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &child)
            .await
            .unwrap(),
        IngestResult::AcceptedPending { .. }
    ));
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &parent)
            .await
            .unwrap(),
        IngestResult::AcceptedReady
    ));
    let pending = group_history::get_received_group_renames(&db, &user, &a.unique_global_id)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].missing, vec![parent.id]);
}

#[tokio::test]
async fn another_owners_pending_reference_cannot_block_a_valid_parent() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let a = group_db::add_empty_group(&db, &user, "A", None)
        .await
        .unwrap();
    let b = group_db::add_empty_group(&db, &other, "B", None)
        .await
        .unwrap();
    let own_replica = random_hex_32();
    let other_replica = random_hex_32();
    let own = trust(&db, &user, &own_replica).await;
    let foreign = trust(&db, &other, &other_replica).await;
    let parent = make_event(&other_replica, 1, &b.unique_global_id, "foreign", vec![]);
    let child = make_event(
        &own_replica,
        1,
        &a.unique_global_id,
        "waiting",
        vec![parent.id.clone()],
    );
    assert!(matches!(
        group_history::ingest_group_rename(&db, &own, &child)
            .await
            .unwrap(),
        IngestResult::AcceptedPending { .. }
    ));
    assert!(matches!(
        group_history::ingest_group_rename(&db, &foreign, &parent)
            .await
            .unwrap(),
        IngestResult::AcceptedReady
    ));
    let pending = group_history::get_received_group_renames(&db, &user, &a.unique_global_id)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].missing, vec![parent.id]);
}

#[tokio::test]
async fn same_replica_parent_must_precede_child_even_when_parent_is_missing() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "local", None)
        .await
        .unwrap();
    let replica = random_hex_32();
    let principal = trust(&db, &user, &replica).await;
    let before = domain_state(&db, &user, group.id, &group.unique_global_id).await;
    for parent_counter in [2, 3] {
        let child = make_event(
            &replica,
            2,
            &group.unique_global_id,
            "invalid",
            vec![EventId {
                replica_id: replica.clone(),
                counter: parent_counter,
            }],
        );
        assert!(
            group_history::ingest_group_rename(&db, &principal, &child)
                .await
                .is_err()
        );
    }
    let other_replica = random_hex_32();
    let child = make_event(
        &replica,
        2,
        &group.unique_global_id,
        "independent replica",
        vec![EventId {
            replica_id: other_replica,
            counter: 100,
        }],
    );
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &child)
            .await
            .unwrap(),
        IngestResult::AcceptedPending { .. }
    ));
    assert_eq!(
        domain_state(&db, &user, group.id, &group.unique_global_id).await,
        before
    );
}

#[tokio::test]
async fn reused_group_identity_rejects_new_events_but_allows_exact_old_replay() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "original", None)
        .await
        .unwrap();
    let replica = random_hex_32();
    let principal = trust(&db, &user, &replica).await;
    let old = make_event(&replica, 1, &group.unique_global_id, "old event", vec![]);
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &old)
            .await
            .unwrap(),
        IngestResult::AcceptedReady
    ));
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(group_db::purge_trashed_groups(&db, 30).await.unwrap(), 1);
    let result = sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id) VALUES (?, ?, ?, ?, ?)")
        .bind("new incarnation").bind("2026-01-01T00:00:00Z").bind(user.id).bind("2026-01-01T00:00:00Z").bind(&group.unique_global_id).execute(&db).await.unwrap();
    let id = result.last_insert_rowid();
    let generation: i64 = sqlx::query_scalar(
        "SELECT generation FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&group.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(generation > 1);
    let before = domain_state(&db, &user, id, &group.unique_global_id).await;
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &old)
            .await
            .unwrap(),
        IngestResult::AlreadyPresent
    ));
    let next = make_event(
        &replica,
        2,
        &group.unique_global_id,
        "ambiguous event",
        vec![old.id],
    );
    assert!(
        group_history::ingest_group_rename(&db, &principal, &next)
            .await
            .is_err()
    );
    assert_eq!(
        domain_state(&db, &user, id, &group.unique_global_id).await,
        before
    );
}
