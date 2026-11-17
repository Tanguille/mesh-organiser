//! Receipt storage is not authentication, causal application, or a sync endpoint.
mod common;
use db::{
    causal_envelope::{self, Action, CausalEnvelope, Dependency, Dot, Entity, EntityKind},
    causal_store::{self, ReceiveResult},
    group_db,
    group_history::ReplicaPrincipal,
    model::user::User,
    random_hex_32, user_db,
};

async fn receive(
    db: &db::db_context::DbContext,
    user: &User,
    principal: &ReplicaPrincipal,
    wire: &str,
) -> Result<ReceiveResult, db::DbError> {
    let identity = user_db::get_sync_identity(db, user.id).await?.unwrap();
    causal_store::receive(db, user, &identity, principal, wire).await
}

async fn get_received(
    db: &db::db_context::DbContext,
    user: &User,
    dot: &Dot,
) -> Result<Option<causal_store::StoredEnvelope>, db::DbError> {
    let identity = user_db::get_sync_identity(db, user.id).await?.unwrap();
    causal_store::get_received(db, user, &identity, dot).await
}

async fn principal(db: &db::db_context::DbContext, user: &User) -> ReplicaPrincipal {
    let writer = random_hex_32();
    sqlx::query("INSERT INTO causal_replica_owners (user_id, replica_id) VALUES (?, ?)")
        .bind(user.id)
        .bind(&writer)
        .execute(db)
        .await
        .unwrap();
    ReplicaPrincipal {
        owner_user_id: user.id,
        replica_id: writer,
    }
}
fn group_entity(global: &str, birth: &str) -> Entity {
    Entity {
        kind: EntityKind::Group,
        global_id: global.into(),
        birth_id: Some(birth.into()),
    }
}
fn dot(writer: &str, count: i64) -> Dot {
    Dot {
        writer_id: writer.into(),
        counter: count.to_string(),
    }
}
fn rename(entity: &Entity, writer: &str, count: i64, name: &str) -> CausalEnvelope {
    CausalEnvelope {
        version: 2,
        entity: entity.clone(),
        event: dot(writer, count),
        parents: vec![],
        dependencies: vec![],
        action: Action::GroupRenamed { name: name.into() },
    }
}
fn wire(event: &CausalEnvelope) -> String {
    causal_envelope::canonical_causal_envelope(event).unwrap()
}
async fn group_fixture(db: &db::db_context::DbContext, user: &User) -> (i64, Entity) {
    let group = group_db::add_empty_group(db, user, "folder", None)
        .await
        .unwrap();
    let birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(db)
            .await
            .unwrap();
    (group.id, group_entity(&group.unique_global_id, &birth))
}

#[tokio::test]
async fn canonical_replay_is_idempotent_but_cross_action_dot_reuse_rejected() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (group_id, entity) = group_fixture(&db, &user).await;
    let writer = principal(&db, &user).await;
    let before: (String, i64) =
        sqlx::query_as("SELECT group_name, group_revision FROM models_group WHERE group_id = ?")
            .bind(group_id)
            .fetch_one(&db)
            .await
            .unwrap();
    let mut event = rename(&entity, &writer.replica_id, 3, "remote");
    event.parents = vec![dot(&writer.replica_id, 2), dot(&writer.replica_id, 1)];
    let result = receive(&db, &user, &writer, &serde_json::to_string(&event).unwrap()).await;
    assert!(
        matches!(result, Ok(ReceiveResult::AcceptedPending { .. })),
        "valid event with missing parents must be retained"
    );
    event.parents.reverse();
    assert!(matches!(
        receive(&db, &user, &writer, &serde_json::to_string(&event).unwrap()).await,
        Ok(ReceiveResult::AlreadyPresent)
    ));
    let stored = get_received(&db, &user, &event.event)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(wire(&stored.envelope), wire(&event));
    assert_eq!(stored.missing.len(), 2);
    let mut changed = event.clone();
    changed.action = Action::GroupDeleted {
        deletion_operation_id: random_hex_32(),
    };
    assert!(receive(&db, &user, &writer, &wire(&changed)).await.is_err());
    let mut wrong_entity = event.clone();
    wrong_entity.entity.birth_id = Some(random_hex_32());
    assert!(
        receive(&db, &user, &writer, &wire(&wrong_entity))
            .await
            .is_err()
    );
    let after: (String, i64) =
        sqlx::query_as("SELECT group_name, group_revision FROM models_group WHERE group_id = ?")
            .bind(group_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(after, before, "receipt alone must never apply the rename");
}

#[tokio::test]
async fn missing_parents_survive_restart_and_resolve_without_applying() {
    let (dir, db) = common::test_db().await;
    let user = User::default();
    let (group_id, entity) = group_fixture(&db, &user).await;
    let writer = principal(&db, &user).await;
    let parent = rename(&entity, &writer.replica_id, 1, "first");
    let mut child = rename(&entity, &writer.replica_id, 2, "second");
    child.parents.push(parent.event.clone());
    assert!(matches!(
        receive(&db, &user, &writer, &wire(&child)).await,
        Ok(ReceiveResult::AcceptedPending { .. })
    ));
    db.close().await;
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    let pending = get_received(&db, &user, &child.event)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        pending.missing,
        vec![Dependency {
            entity: entity.clone(),
            event: parent.event.clone()
        }]
    );
    assert!(matches!(
        receive(&db, &user, &writer, &wire(&parent)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    let completed = get_received(&db, &user, &child.event)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(completed.missing, []);
    assert_eq!(completed.envelope, child);
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id = ?")
        .bind(group_id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(name, "folder");
}

#[tokio::test]
async fn owner_and_retired_local_writer_are_isolated_while_valid_remote_works() {
    let (dir, db) = common::test_db().await;
    let user = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let (_, entity) = group_fixture(&db, &user).await;
    let old_writer: String =
        sqlx::query_scalar("SELECT replica_id FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    db.close().await;
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    let current: String =
        sqlx::query_scalar("SELECT replica_id FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_ne!(old_writer, current);
    let old_principal = ReplicaPrincipal {
        owner_user_id: user.id,
        replica_id: old_writer.clone(),
    };
    let registered =
        sqlx::query("INSERT INTO causal_replica_owners (user_id, replica_id) VALUES (?, ?)")
            .bind(user.id)
            .bind(&old_writer)
            .execute(&db)
            .await;
    if registered.is_ok() {
        let forged = rename(&entity, &old_writer, 999, "forged");
        assert!(
            receive(&db, &user, &old_principal, &wire(&forged))
                .await
                .is_err()
        );
    }
    let good = principal(&db, &user).await;
    let event = rename(&entity, &good.replica_id, 1, "remote");
    assert!(matches!(
        receive(&db, &user, &good, &wire(&event)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    assert!(
        get_received(&db, &other, &event.event)
            .await
            .unwrap()
            .is_none()
    );
    assert!(receive(&db, &other, &good, &wire(&event)).await.is_err());
}

#[tokio::test]
async fn failed_dependency_insert_rolls_back_event_and_retry_succeeds() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (_, entity) = group_fixture(&db, &user).await;
    let writer = principal(&db, &user).await;
    let mut event = rename(&entity, &writer.replica_id, 2, "pending");
    event.parents.push(dot(&writer.replica_id, 1));
    sqlx::query("CREATE TRIGGER reject_causal_dependency BEFORE INSERT ON causal_receipt_dependencies BEGIN SELECT RAISE(ABORT, 'injected failure'); END").execute(&db).await.unwrap();
    assert!(receive(&db, &user, &writer, &wire(&event)).await.is_err());
    assert!(
        get_received(&db, &user, &event.event)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("DROP TRIGGER reject_causal_dependency")
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(
        receive(&db, &user, &writer, &wire(&event)).await,
        Ok(ReceiveResult::AcceptedPending { .. })
    ));
}

#[tokio::test]
async fn cycles_are_rejected_without_losing_the_prior_pending_event() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (_, entity) = group_fixture(&db, &user).await;
    let first = principal(&db, &user).await;
    let second = principal(&db, &user).await;
    let mut a = rename(&entity, &first.replica_id, 1, "a");
    let mut b = rename(&entity, &second.replica_id, 1, "b");
    a.parents.push(b.event.clone());
    b.parents.push(a.event.clone());
    assert!(matches!(
        receive(&db, &user, &first, &wire(&a)).await,
        Ok(ReceiveResult::AcceptedPending { .. })
    ));
    assert!(receive(&db, &user, &second, &wire(&b)).await.is_err());
    let retained = get_received(&db, &user, &a.event).await.unwrap();
    assert!(retained.is_some());
    assert_eq!(
        retained.unwrap().missing,
        vec![Dependency {
            entity: entity.clone(),
            event: b.event.clone()
        }]
    );
    b.parents.clear();
    assert!(matches!(
        receive(&db, &user, &second, &wire(&b)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    assert_eq!(
        get_received(&db, &user, &a.event)
            .await
            .unwrap()
            .unwrap()
            .missing,
        []
    );
}

#[tokio::test]
async fn poisoned_pending_child_does_not_block_parent_of_another_entity() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (_, a) = group_fixture(&db, &user).await;
    let (_, b) = group_fixture(&db, &user).await;
    let first = principal(&db, &user).await;
    let second = principal(&db, &user).await;
    let parent = rename(&b, &second.replica_id, 1, "valid parent");
    let mut child = rename(&a, &first.replica_id, 1, "poisoned");
    child.parents.push(parent.event.clone());
    assert!(matches!(
        receive(&db, &user, &first, &wire(&child)).await,
        Ok(ReceiveResult::AcceptedPending { .. })
    ));
    assert!(matches!(
        receive(&db, &user, &second, &wire(&parent)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    let stored = get_received(&db, &user, &child.event).await;
    assert!(
        stored.is_err() || stored.unwrap().is_some_and(|item| !item.missing.is_empty()),
        "wrong-entity dot must not make the child ready"
    );
    let mut another = rename(&a, &first.replica_id, 2, "mismatch");
    another.parents.push(parent.event);
    assert!(receive(&db, &user, &first, &wire(&another)).await.is_err());
}

#[tokio::test]
async fn foreign_pending_claim_does_not_block_a_valid_owner_event() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let (_, owned_entity) = group_fixture(&db, &owner).await;
    let (_, foreign_entity) = group_fixture(&db, &other).await;
    let writer = principal(&db, &owner).await;
    let foreign_writer = principal(&db, &other).await;
    let parent = rename(&owned_entity, &writer.replica_id, 1, "owner event");
    let mut claim = rename(&foreign_entity, &foreign_writer.replica_id, 1, "claim");
    claim.parents.push(parent.event.clone());
    assert!(matches!(
        receive(&db, &other, &foreign_writer, &wire(&claim)).await,
        Ok(ReceiveResult::AcceptedPending { .. })
    ));
    assert!(matches!(
        receive(&db, &owner, &writer, &wire(&parent)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    assert!(
        get_received(&db, &other, &parent.event)
            .await
            .unwrap()
            .is_none()
    );
    let claim_state = get_received(&db, &other, &claim.event).await;
    assert!(
        claim_state.is_err()
            || claim_state
                .unwrap()
                .is_some_and(|item| !item.missing.is_empty())
    );
    let mut invalid = rename(&foreign_entity, &foreign_writer.replica_id, 2, "invalid");
    invalid.parents.push(parent.event);
    assert!(
        receive(&db, &other, &foreign_writer, &wire(&invalid))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn owner_event_quota_rejects_new_events_but_allows_replay_and_other_owners() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let (_, entity) = group_fixture(&db, &owner).await;
    let writer = principal(&db, &owner).await;
    // Seed canonical receipt rows in one transaction to reach the real production boundary
    // without a test-only quota override.
    let mut tx = db.begin().await.unwrap();
    for counter in 1..=causal_store::MAX_OWNER_EVENTS {
        let counter = i64::try_from(counter).unwrap();
        let event = rename(&entity, &writer.replica_id, counter, "seed");
        sqlx::query(
            "INSERT INTO causal_events (user_id, writer_id, counter, envelope) VALUES (?, ?, ?, ?)",
        )
        .bind(owner.id)
        .bind(&writer.replica_id)
        .bind(counter)
        .bind(wire(&event))
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO causal_event_receipts (user_id, writer_id, counter) VALUES (?, ?, ?)",
        )
        .bind(owner.id)
        .bind(&writer.replica_id)
        .bind(counter)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let replay = rename(&entity, &writer.replica_id, 1, "seed");
    assert!(matches!(
        receive(&db, &owner, &writer, &wire(&replay)).await,
        Ok(ReceiveResult::AlreadyPresent)
    ));
    let next = rename(
        &entity,
        &writer.replica_id,
        i64::try_from(causal_store::MAX_OWNER_EVENTS).unwrap() + 1,
        "over quota",
    );
    assert!(receive(&db, &owner, &writer, &wire(&next)).await.is_err());
    assert!(
        get_received(&db, &owner, &next.event)
            .await
            .unwrap()
            .is_none()
    );
    let other_id = user_db::add_user(&db, "other", "other-quota@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let (_, other_entity) = group_fixture(&db, &other).await;
    let other_writer = principal(&db, &other).await;
    let event = rename(&other_entity, &other_writer.replica_id, 1, "under quota");
    assert!(matches!(
        receive(&db, &other, &other_writer, &wire(&event)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
}

#[tokio::test]
async fn cross_entity_dependencies_wait_and_reject_the_wrong_event_action() {
    use db::blob_db;
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (_, group) = group_fixture(&db, &user).await;
    let group_birth = group.birth_id.as_deref().unwrap();
    let creation =
        causal_envelope::get_local_creation_record(&db, &user, EntityKind::Group, group_birth)
            .await
            .unwrap()
            .unwrap()
            .envelope
            .event;
    let blob = blob_db::add_blob(&db, "cross-dependency", "stl", 1, None)
        .await
        .unwrap();
    let model_id = db::model_db::add_model(&db, &user, "part", blob, None, None)
        .await
        .unwrap();
    let (global, birth): (String, String) = sqlx::query_as(
        "SELECT model_unique_global_id, model_birth_id FROM models WHERE model_id = ?",
    )
    .bind(model_id)
    .fetch_one(&db)
    .await
    .unwrap();
    let model = Entity {
        kind: EntityKind::Model,
        global_id: global,
        birth_id: Some(birth),
    };
    let group_writer = principal(&db, &user).await;
    let model_writer = principal(&db, &user).await;
    let mut movement = CausalEnvelope {
        version: 2,
        entity: model.clone(),
        event: dot(&model_writer.replica_id, 1),
        parents: vec![],
        dependencies: vec![Dependency {
            entity: group.clone(),
            event: creation.clone(),
        }],
        action: Action::ModelMoved {
            from: None,
            to: Some(group.clone()),
            target_lifecycle: Some(creation.clone()),
        },
    };
    let add = CausalEnvelope {
        version: 2,
        entity: group.clone(),
        event: dot(&group_writer.replica_id, 1),
        parents: vec![],
        dependencies: vec![Dependency {
            entity: model.clone(),
            event: movement.event.clone(),
        }],
        action: Action::GroupMemberAdded {
            model: model.clone(),
            membership_event: movement.event.clone(),
        },
    };
    assert!(matches!(
        receive(&db, &user, &group_writer, &wire(&add)).await,
        Ok(ReceiveResult::AcceptedPending { .. })
    ));
    assert!(matches!(
        receive(&db, &user, &model_writer, &wire(&movement)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    assert_eq!(
        get_received(&db, &user, &add.event)
            .await
            .unwrap()
            .unwrap()
            .missing,
        []
    );
    movement.event = dot(&model_writer.replica_id, 2);
    movement.action = Action::ModelDetached {
        group: group.clone(),
        deletion: creation.clone(),
    };
    assert!(
        receive(&db, &user, &model_writer, &wire(&movement))
            .await
            .is_err(),
        "a group creation dot cannot masquerade as a deletion"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_admission_at_event_quota_has_exactly_one_winner() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (_, entity) = group_fixture(&db, &user).await;
    let writer = principal(&db, &user).await;
    let mut tx = db.begin().await.unwrap();
    for counter in 1..causal_store::MAX_OWNER_EVENTS {
        let counter = i64::try_from(counter).unwrap();
        let event = rename(&entity, &writer.replica_id, counter, "seed");
        sqlx::query(
            "INSERT INTO causal_events (user_id, writer_id, counter, envelope) VALUES (?, ?, ?, ?)",
        )
        .bind(user.id)
        .bind(&writer.replica_id)
        .bind(counter)
        .bind(wire(&event))
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO causal_event_receipts (user_id, writer_id, counter) VALUES (?, ?, ?)",
        )
        .bind(user.id)
        .bind(&writer.replica_id)
        .bind(counter)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let counter = i64::try_from(causal_store::MAX_OWNER_EVENTS).unwrap();
    let first = rename(&entity, &writer.replica_id, counter, "first");
    let second = rename(&entity, &writer.replica_id, counter + 1, "second");
    let first_db = db.clone();
    let second_db = db.clone();
    let first_user = user.clone();
    let second_user = user.clone();
    let first_principal = writer.clone();
    let second_principal = writer.clone();
    let first = tokio::spawn(async move {
        receive(&first_db, &first_user, &first_principal, &wire(&first)).await
    });
    let second = tokio::spawn(async move {
        receive(&second_db, &second_user, &second_principal, &wire(&second)).await
    });
    let first = first.await.unwrap();
    let second = second.await.unwrap();
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM causal_event_receipts WHERE user_id = ?")
            .bind(user.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(
        count,
        i64::try_from(causal_store::MAX_OWNER_EVENTS).unwrap()
    );
}

#[tokio::test]
async fn owner_byte_quota_blocks_new_receipts_but_not_exact_replays() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (_, entity) = group_fixture(&db, &user).await;
    let writer = principal(&db, &user).await;
    let mut tx = db.begin().await.unwrap();
    let mut bytes = 0_usize;
    let mut counter = 1_i64;
    loop {
        let event = rename(
            &entity,
            &writer.replica_id,
            counter,
            &"x".repeat(causal_envelope::MAX_NAME_BYTES),
        );
        let canonical = wire(&event);
        if bytes + canonical.len() > causal_store::MAX_OWNER_BYTES {
            break;
        }
        bytes += canonical.len();
        sqlx::query(
            "INSERT INTO causal_events (user_id, writer_id, counter, envelope) VALUES (?, ?, ?, ?)",
        )
        .bind(user.id)
        .bind(&writer.replica_id)
        .bind(counter)
        .bind(canonical)
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO causal_event_receipts (user_id, writer_id, counter) VALUES (?, ?, ?)",
        )
        .bind(user.id)
        .bind(&writer.replica_id)
        .bind(counter)
        .execute(&mut *tx)
        .await
        .unwrap();
        counter += 1;
    }
    tx.commit().await.unwrap();
    assert!(counter > 1);
    let replay = rename(
        &entity,
        &writer.replica_id,
        1,
        &"x".repeat(causal_envelope::MAX_NAME_BYTES),
    );
    assert!(matches!(
        receive(&db, &user, &writer, &wire(&replay)).await,
        Ok(ReceiveResult::AlreadyPresent)
    ));
    let next = rename(
        &entity,
        &writer.replica_id,
        counter,
        &"x".repeat(causal_envelope::MAX_NAME_BYTES),
    );
    assert!(bytes + wire(&next).len() > causal_store::MAX_OWNER_BYTES);
    assert!(receive(&db, &user, &writer, &wire(&next)).await.is_err());
    assert!(
        get_received(&db, &user, &next.event)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn owner_dependency_quota_accepts_boundary_and_rejects_next_edge() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (_, entity) = group_fixture(&db, &user).await;
    let writer = principal(&db, &user).await;
    let mut tx = db.begin().await.unwrap();
    let mut left = causal_store::MAX_OWNER_DEPENDENCIES - 1;
    let mut counter = 100_000_i64;
    while left > 0 {
        let count = left.min(causal_envelope::MAX_CONTEXT_ITEMS);
        let mut event = rename(&entity, &writer.replica_id, counter, "seed");
        event.parents = (1..=count)
            .map(|n| dot(&writer.replica_id, i64::try_from(n).unwrap()))
            .collect();
        sqlx::query(
            "INSERT INTO causal_events (user_id, writer_id, counter, envelope) VALUES (?, ?, ?, ?)",
        )
        .bind(user.id)
        .bind(&writer.replica_id)
        .bind(counter)
        .bind(wire(&event))
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO causal_event_receipts (user_id, writer_id, counter) VALUES (?, ?, ?)",
        )
        .bind(user.id)
        .bind(&writer.replica_id)
        .bind(counter)
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query("WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < ?) INSERT INTO causal_receipt_dependencies (user_id, writer_id, counter, dependency_writer_id, dependency_counter) SELECT ?, ?, ?, ?, n FROM seq")
            .bind(i64::try_from(count).unwrap()).bind(user.id).bind(&writer.replica_id).bind(counter).bind(&writer.replica_id).execute(&mut *tx).await.unwrap();
        counter += 1;
        left -= count;
    }
    tx.commit().await.unwrap();
    let mut boundary = rename(&entity, &writer.replica_id, counter, "boundary");
    boundary.parents.push(dot(&writer.replica_id, 50_000));
    assert!(matches!(
        receive(&db, &user, &writer, &wire(&boundary)).await,
        Ok(ReceiveResult::AcceptedPending { .. })
    ));
    assert!(matches!(
        receive(&db, &user, &writer, &wire(&boundary)).await,
        Ok(ReceiveResult::AlreadyPresent)
    ));
    let mut next = rename(&entity, &writer.replica_id, counter + 1, "over quota");
    next.parents.push(dot(&writer.replica_id, 50_001));
    assert!(receive(&db, &user, &writer, &wire(&next)).await.is_err());
    assert!(
        get_received(&db, &user, &next.event)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn model_move_accepts_verified_birth_legacy_group_observation_without_claiming_complete_history()
 {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (_, group) = group_fixture(&db, &user).await;
    let blob = db::blob_db::add_blob(&db, "legacy-observation", "stl", 1, None)
        .await
        .unwrap();
    let model_id = db::model_db::add_model(&db, &user, "part", blob, None, None)
        .await
        .unwrap();
    let (global_id, birth): (String, String) = sqlx::query_as(
        "SELECT model_unique_global_id, model_birth_id FROM models WHERE model_id = ?",
    )
    .bind(model_id)
    .fetch_one(&db)
    .await
    .unwrap();
    let model = Entity {
        kind: EntityKind::Model,
        global_id,
        birth_id: Some(birth),
    };
    let group_writer = principal(&db, &user).await;
    let model_writer = principal(&db, &user).await;
    let observation = CausalEnvelope {
        version: 2,
        entity: group.clone(),
        event: dot(&group_writer.replica_id, 1),
        parents: vec![],
        dependencies: vec![],
        action: Action::LegacyObserved {
            observation_digest: "unknown-ancestry-evidence".into(),
        },
    };
    assert!(matches!(
        receive(&db, &user, &group_writer, &wire(&observation)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    let movement = CausalEnvelope {
        version: 2,
        entity: model,
        event: dot(&model_writer.replica_id, 1),
        parents: vec![],
        dependencies: vec![Dependency {
            entity: group.clone(),
            event: observation.event.clone(),
        }],
        action: Action::ModelMoved {
            from: None,
            to: Some(group.clone()),
            target_lifecycle: Some(observation.event),
        },
    };
    assert!(
        matches!(
            receive(&db, &user, &model_writer, &wire(&movement)).await,
            Ok(ReceiveResult::AcceptedReady)
        ),
        "a verified-birth legacy observation is a valid target lifecycle dependency"
    );
    let stored = get_received(&db, &user, &movement.event)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.missing, []);
    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
            .bind(model_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(assigned, None, "receipt readiness does not apply the move");
    let history = db::group_history::get_group_history(&db, &user, &group.global_id)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        history.ancestry,
        db::group_history::HistoryOrigin::UnknownPreUpgrade
    ));
}

#[tokio::test]
async fn local_creation_dependencies_cannot_bypass_total_graph_byte_limit() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (_, root_entity) = group_fixture(&db, &user).await;
    let writer = principal(&db, &user).await;
    let creation_writer = random_hex_32();
    let mut dependencies = Vec::new();
    let mut bytes = 0_usize;
    let mut counter = 1_i64;
    let mut tx = db.begin().await.unwrap();
    while bytes <= causal_store::MAX_OWNER_BYTES {
        let entity = group_entity(&random_hex_32(), &random_hex_32());
        let creation = CausalEnvelope {
            version: 2,
            entity: entity.clone(),
            event: dot(&creation_writer, counter),
            parents: vec![],
            dependencies: vec![],
            action: Action::GroupCreated {
                name: "x".repeat(causal_envelope::MAX_NAME_BYTES),
            },
        };
        let canonical = wire(&creation);
        bytes += canonical.len();
        sqlx::query("INSERT INTO entity_births (user_id, entity_kind, birth_id, global_id) VALUES (?, 'group', ?, ?)")
            .bind(user.id).bind(entity.birth_id.as_ref().unwrap()).bind(&entity.global_id).execute(&mut *tx).await.unwrap();
        sqlx::query(
            "INSERT INTO causal_events (user_id, writer_id, counter, envelope) VALUES (?, ?, ?, ?)",
        )
        .bind(user.id)
        .bind(&creation_writer)
        .bind(counter)
        .bind(canonical)
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query("INSERT INTO local_creation_records (user_id, entity_kind, birth_id, writer_id, counter) VALUES (?, 'group', ?, ?, ?)")
            .bind(user.id).bind(entity.birth_id.as_ref().unwrap()).bind(&creation_writer).bind(counter).execute(&mut *tx).await.unwrap();
        dependencies.push(Dependency {
            entity,
            event: creation.event,
        });
        counter += 1;
    }
    tx.commit().await.unwrap();
    assert!(dependencies.len() <= causal_envelope::MAX_CONTEXT_ITEMS);
    let event = CausalEnvelope {
        version: 2,
        entity: root_entity,
        event: dot(&writer.replica_id, 1),
        parents: vec![],
        dependencies,
        action: Action::LegacyObserved {
            observation_digest: "bounded graph".into(),
        },
    };
    assert!(
        receive(&db, &user, &writer, &wire(&event)).await.is_err(),
        "locally stored creation evidence must count toward the graph traversal limit"
    );
    assert!(
        get_received(&db, &user, &event.event)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn stale_account_identity_cannot_read_or_write_after_numeric_id_reuse() {
    let (_dir, db) = common::test_db().await;
    let id = user_db::add_user(&db, "old", "old@example.test", &random_hex_32())
        .await
        .unwrap();
    let old = user_db::get_user_by_id(&db, id).await.unwrap().unwrap();
    let old_identity = user_db::get_sync_identity(&db, id).await.unwrap().unwrap();
    let writer = random_hex_32();
    sqlx::query("INSERT INTO causal_replica_owners (user_id, replica_id) VALUES (?, ?)")
        .bind(id)
        .bind(&writer)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM users WHERE user_id = ?")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (user_id, user_name, user_email, user_password_hash, user_created_at) VALUES (?, 'new', 'new@example.test', 'unused', '2020-01-01T00:00:00Z')").bind(id).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO causal_replica_owners (user_id, replica_id) VALUES (?, ?)")
        .bind(id)
        .bind(&writer)
        .execute(&db)
        .await
        .unwrap();
    let new_user = user_db::get_user_by_id(&db, id).await.unwrap().unwrap();
    let current = user_db::get_sync_identity(&db, id).await.unwrap().unwrap();
    assert_ne!(old_identity.account_id, current.account_id);
    let (_, entity) = group_fixture(&db, &new_user).await;
    let principal = ReplicaPrincipal {
        owner_user_id: id,
        replica_id: writer.clone(),
    };
    let first = rename(&entity, &writer, 1, "new account event");
    assert!(matches!(
        causal_store::receive(&db, &new_user, &current, &principal, &wire(&first)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    assert!(matches!(
        causal_store::receive(&db, &new_user, &current, &principal, &wire(&first)).await,
        Ok(ReceiveResult::AlreadyPresent)
    ));
    assert!(
        causal_store::get_received(&db, &old, &old_identity, &first.event)
            .await
            .is_err(),
        "old account identity cannot read replacement receipts"
    );
    let second = rename(&entity, &writer, 2, "stale write");
    assert!(
        causal_store::receive(&db, &old, &old_identity, &principal, &wire(&second))
            .await
            .is_err(),
        "old account identity cannot write to replacement account"
    );
    let mut wrong_instance = current.clone();
    wrong_instance.instance_id = random_hex_32();
    assert!(
        causal_store::get_received(&db, &new_user, &wrong_instance, &first.event)
            .await
            .is_err()
    );
    assert!(
        causal_store::receive(&db, &new_user, &wrong_instance, &principal, &wire(&second))
            .await
            .is_err()
    );
    assert!(
        causal_store::get_received(&db, &new_user, &current, &second.event)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn replacement_committing_before_receipt_write_rejects_stale_identity() {
    use std::time::Duration;
    let (_dir, db) = common::test_db().await;
    let id = user_db::add_user(&db, "old", "race-old@example.test", &random_hex_32())
        .await
        .unwrap();
    let old = user_db::get_user_by_id(&db, id).await.unwrap().unwrap();
    let identity = user_db::get_sync_identity(&db, id).await.unwrap().unwrap();
    let (_, entity) = group_fixture(&db, &old).await;
    let principal = principal(&db, &old).await;
    let event = rename(&entity, &principal.replica_id, 1, "stale");
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("DELETE FROM users WHERE user_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (user_id, user_name, user_email, user_password_hash, user_created_at) VALUES (?, 'new', 'race-new@example.test', 'unused', '2020-01-01T00:00:00Z')").bind(id).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO causal_replica_owners (user_id, replica_id) VALUES (?, ?)")
        .bind(id)
        .bind(&principal.replica_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    let worker_db = db.clone();
    let worker_user = old.clone();
    let worker_identity = identity.clone();
    let worker_principal = principal.clone();
    let payload = wire(&event);
    let mut task = tokio::spawn(async move {
        causal_store::receive(
            &worker_db,
            &worker_user,
            &worker_identity,
            &worker_principal,
            &payload,
        )
        .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut task)
            .await
            .is_err(),
        "write must wait for the competing transaction"
    );
    tx.commit().await.unwrap();
    assert!(
        task.await.unwrap().is_err(),
        "identity must be checked after the replacement commits"
    );
    let current = user_db::get_sync_identity(&db, id).await.unwrap().unwrap();
    let current_user = user_db::get_user_by_id(&db, id).await.unwrap().unwrap();
    assert!(
        causal_store::get_received(&db, &current_user, &current, &event.event)
            .await
            .unwrap()
            .is_none()
    );
}
