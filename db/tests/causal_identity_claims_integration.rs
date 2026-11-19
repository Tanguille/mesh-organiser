//! Conflicting remote identity claims remain visible and unapplied, regardless of arrival order.
use db::{
    causal_envelope::{Action, CausalEnvelope, Dependency, Dot, Entity, EntityKind},
    causal_store::{self, IdentityStatus, ReceiveResult},
    db_context::DbContext,
    group_db,
    group_history::ReplicaPrincipal,
    model::user::User,
    model_db, random_hex_32, user_db,
};

mod common;

async fn principal(db: &DbContext, user: &User) -> ReplicaPrincipal {
    let replica_id = random_hex_32();
    sqlx::query("INSERT INTO causal_replica_owners (user_id, replica_id) VALUES (?, ?)")
        .bind(user.id)
        .bind(&replica_id)
        .execute(db)
        .await
        .unwrap();
    ReplicaPrincipal {
        owner_user_id: user.id,
        replica_id,
    }
}
fn entity(kind: EntityKind, global: &str, birth: &str) -> Entity {
    Entity {
        kind,
        global_id: global.into(),
        birth_id: Some(birth.into()),
    }
}
fn created(entity: Entity, principal: &ReplicaPrincipal, counter: i64) -> CausalEnvelope {
    let action = match entity.kind {
        EntityKind::Group => Action::GroupCreated {
            name: "remote".into(),
        },
        EntityKind::Model => Action::ModelCreated,
    };
    CausalEnvelope {
        version: 2,
        entity,
        event: Dot {
            writer_id: principal.replica_id.clone(),
            counter: counter.to_string(),
        },
        parents: vec![],
        dependencies: vec![],
        action,
    }
}
fn wire(event: &CausalEnvelope) -> String {
    db::causal_envelope::canonical_causal_envelope(event).unwrap()
}
async fn status(
    db: &DbContext,
    user: &User,
    identity: &user_db::SyncIdentity,
    event: &CausalEnvelope,
) -> IdentityStatus {
    causal_store::get_received(db, user, identity, &event.event)
        .await
        .unwrap()
        .unwrap()
        .identity_status
}

#[tokio::test]
async fn same_global_different_births_are_both_retained_and_recomputed_as_conflicts() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let first_writer = principal(&db, &user).await;
    let second_writer = principal(&db, &user).await;
    let global = random_hex_32();
    let first = created(
        entity(EntityKind::Group, &global, &random_hex_32()),
        &first_writer,
        1,
    );
    let second = created(
        entity(EntityKind::Group, &global, &random_hex_32()),
        &second_writer,
        1,
    );
    let initial = causal_store::receive(&db, &user, &identity, &first_writer, &wire(&first)).await;
    assert!(
        matches!(initial, Ok(ReceiveResult::AcceptedReady)),
        "first valid claim should be retained"
    );
    assert_eq!(
        status(&db, &user, &identity, &first).await,
        IdentityStatus::Uncontested
    );
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &second_writer, &wire(&second)).await,
        Ok(ReceiveResult::AcceptedIdentityConflict { .. })
    ));
    assert_eq!(
        status(&db, &user, &identity, &first).await,
        IdentityStatus::Conflict
    );
    assert_eq!(
        status(&db, &user, &identity, &second).await,
        IdentityStatus::Conflict
    );
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &first_writer, &wire(&first)).await,
        Ok(ReceiveResult::AlreadyPresent)
    ));
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM causal_event_receipts WHERE user_id = ?")
            .bind(user.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(count, 2);
    let domain_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM models_group WHERE group_user_id = ?")
            .bind(user.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(
        domain_count, 0,
        "identity claims are receipts, not applied groups"
    );
}

#[tokio::test]
async fn same_birth_different_global_ids_conflict_but_identical_claims_do_not() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let first_principal = principal(&db, &user).await;
    let second = principal(&db, &user).await;
    let third = principal(&db, &user).await;
    let global = random_hex_32();
    let birth = random_hex_32();
    let first = created(
        entity(EntityKind::Group, &global, &birth),
        &first_principal,
        1,
    );
    let same = created(entity(EntityKind::Group, &global, &birth), &second, 1);
    let other = created(
        entity(EntityKind::Group, &random_hex_32(), &birth),
        &third,
        1,
    );
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &first_principal, &wire(&first)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &second, &wire(&same)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    assert_eq!(
        status(&db, &user, &identity, &first).await,
        IdentityStatus::Uncontested
    );
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &third, &wire(&other)).await,
        Ok(ReceiveResult::AcceptedIdentityConflict { .. })
    ));
    for event in [&first, &same, &other] {
        assert_eq!(
            status(&db, &user, &identity, event).await,
            IdentityStatus::Conflict
        );
    }
}

#[tokio::test]
async fn conflict_against_local_birth_is_retained_without_mutating_local_data() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &user, "local", None)
        .await
        .unwrap();
    let writer = principal(&db, &user).await;
    let event = created(
        entity(EntityKind::Group, &group.unique_global_id, &random_hex_32()),
        &writer,
        1,
    );
    let received = causal_store::receive(&db, &user, &identity, &writer, &wire(&event)).await;
    assert!(
        matches!(received, Ok(ReceiveResult::AcceptedIdentityConflict { .. })),
        "conflicting remote evidence must be retained, not discarded"
    );
    assert_eq!(
        status(&db, &user, &identity, &event).await,
        IdentityStatus::Conflict
    );
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id = ?")
        .bind(group.id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(name, "local");
}

#[tokio::test]
async fn owner_and_entity_kind_have_independent_identity_namespaces() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let foreign_id = user_db::add_user(
        &db,
        "foreign",
        "claims-foreign@example.test",
        &random_hex_32(),
    )
    .await
    .unwrap();
    let foreign = user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let owner_id = user_db::get_sync_identity(&db, owner.id)
        .await
        .unwrap()
        .unwrap();
    let foreign_identity = user_db::get_sync_identity(&db, foreign.id)
        .await
        .unwrap()
        .unwrap();
    let one = principal(&db, &owner).await;
    let two = principal(&db, &owner).await;
    let three = principal(&db, &foreign).await;
    let global = random_hex_32();
    let birth = random_hex_32();
    let owner_group = created(entity(EntityKind::Group, &global, &birth), &one, 1);
    let owner_model = created(
        entity(EntityKind::Model, &global, &random_hex_32()),
        &two,
        1,
    );
    let foreign_group = created(
        entity(EntityKind::Group, &global, &random_hex_32()),
        &three,
        1,
    );
    for (user, identity, principal, event) in [
        (&owner, &owner_id, &one, &owner_group),
        (&owner, &owner_id, &two, &owner_model),
        (&foreign, &foreign_identity, &three, &foreign_group),
    ] {
        assert!(matches!(
            causal_store::receive(&db, user, identity, principal, &wire(event)).await,
            Ok(ReceiveResult::AcceptedReady)
        ));
        assert_eq!(
            status(&db, user, identity, event).await,
            IdentityStatus::Uncontested
        );
    }
    assert!(
        causal_store::get_received(&db, &foreign, &foreign_identity, &owner_group.event)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_conflicting_claims_are_both_retained_without_first_arrival_winning() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let first_principal = principal(&db, &user).await;
    let second_principal = principal(&db, &user).await;
    let global = random_hex_32();
    let first = created(
        entity(EntityKind::Group, &global, &random_hex_32()),
        &first_principal,
        1,
    );
    let second = created(
        entity(EntityKind::Group, &global, &random_hex_32()),
        &second_principal,
        1,
    );
    let db_a = db.clone();
    let db_b = db.clone();
    let user_a = user.clone();
    let user_b = user.clone();
    let identity_a = identity.clone();
    let identity_b = identity.clone();
    let first_wire = wire(&first);
    let second_wire = wire(&second);
    let task_a = tokio::spawn(async move {
        causal_store::receive(&db_a, &user_a, &identity_a, &first_principal, &first_wire).await
    });
    let task_b = tokio::spawn(async move {
        causal_store::receive(&db_b, &user_b, &identity_b, &second_principal, &second_wire).await
    });
    assert!(task_a.await.unwrap().is_ok());
    assert!(task_b.await.unwrap().is_ok());
    assert_eq!(
        status(&db, &user, &identity, &first).await,
        IdentityStatus::Conflict
    );
    assert_eq!(
        status(&db, &user, &identity, &second).await,
        IdentityStatus::Conflict
    );
}

#[tokio::test]
async fn conflicted_dependency_does_not_make_a_dependent_event_ready() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let first_principal = principal(&db, &user).await;
    let second = principal(&db, &user).await;
    let model_writer = principal(&db, &user).await;
    let global = random_hex_32();
    let group = entity(EntityKind::Group, &global, &random_hex_32());
    let first = created(group.clone(), &first_principal, 1);
    let conflict = created(
        entity(EntityKind::Group, &global, &random_hex_32()),
        &second,
        1,
    );
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &first_principal, &wire(&first)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &second, &wire(&conflict)).await,
        Ok(ReceiveResult::AcceptedIdentityConflict { .. })
    ));
    let model = entity(EntityKind::Model, &random_hex_32(), &random_hex_32());
    let child = CausalEnvelope {
        version: 2,
        entity: model,
        event: Dot {
            writer_id: model_writer.replica_id.clone(),
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![Dependency {
            entity: group.clone(),
            event: first.event.clone(),
        }],
        action: Action::ModelMoved {
            from: None,
            to: Some(group),
            target_lifecycle: Some(first.event.clone()),
        },
    };
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &model_writer, &wire(&child)).await,
        Ok(ReceiveResult::AcceptedIdentityConflict { .. })
    ));
    assert_eq!(
        status(&db, &user, &identity, &child).await,
        IdentityStatus::Conflict
    );
}

#[tokio::test]
async fn unknown_birth_legacy_observation_is_not_marked_verified_or_uncontested() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group_writer = principal(&db, &user).await;
    let model_writer = principal(&db, &user).await;
    let global = random_hex_32();
    let unknown = Entity {
        kind: EntityKind::Group,
        global_id: global.clone(),
        birth_id: None,
    };
    let event = CausalEnvelope {
        version: 2,
        entity: unknown,
        event: Dot {
            writer_id: group_writer.replica_id.clone(),
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![],
        action: Action::LegacyObserved {
            observation_digest: "legacy evidence".into(),
        },
    };
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &group_writer, &wire(&event)).await,
        Ok(ReceiveResult::AcceptedIdentityUnknown { .. })
    ));
    assert_eq!(
        status(&db, &user, &identity, &event).await,
        IdentityStatus::Unknown
    );
    let forged_group = entity(EntityKind::Group, &global, &random_hex_32());
    let child = CausalEnvelope {
        version: 2,
        entity: entity(EntityKind::Model, &random_hex_32(), &random_hex_32()),
        event: Dot {
            writer_id: model_writer.replica_id.clone(),
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![Dependency {
            entity: forged_group.clone(),
            event: event.event.clone(),
        }],
        action: Action::ModelMoved {
            from: None,
            to: Some(forged_group),
            target_lifecycle: Some(event.event.clone()),
        },
    };
    assert!(
        causal_store::receive(&db, &user, &identity, &model_writer, &wire(&child))
            .await
            .is_err(),
        "an unknown-birth event cannot establish a claimed birth"
    );
    assert_eq!(
        status(&db, &user, &identity, &event).await,
        IdentityStatus::Unknown
    );
}

#[tokio::test]
async fn unverified_dependency_reference_does_not_poison_an_authored_identity_claim() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let first_writer = principal(&db, &user).await;
    let child_writer = principal(&db, &user).await;
    let global = random_hex_32();
    let first = created(
        entity(EntityKind::Group, &global, &random_hex_32()),
        &first_writer,
        1,
    );
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &first_writer, &wire(&first)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    let forged_reference = entity(EntityKind::Group, &global, &random_hex_32());
    let child = CausalEnvelope {
        version: 2,
        entity: entity(EntityKind::Model, &random_hex_32(), &random_hex_32()),
        event: Dot {
            writer_id: child_writer.replica_id.clone(),
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![Dependency {
            entity: forged_reference.clone(),
            event: Dot {
                writer_id: random_hex_32(),
                counter: "1".into(),
            },
        }],
        action: Action::ModelMoved {
            from: None,
            to: Some(forged_reference),
            target_lifecycle: Some(Dot {
                writer_id: random_hex_32(),
                counter: "1".into(),
            }),
        },
    };
    // The action must point to the same missing dot as the dependency.
    let mut child = child;
    if let Action::ModelMoved {
        target_lifecycle, ..
    } = &mut child.action
    {
        *target_lifecycle = Some(child.dependencies[0].event.clone());
    }
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &child_writer, &wire(&child)).await,
        Ok(ReceiveResult::AcceptedPending { .. })
    ));
    assert_eq!(
        status(&db, &user, &identity, &first).await,
        IdentityStatus::Uncontested
    );
}

#[tokio::test]
async fn known_claim_against_local_legacy_birth_is_unknown_not_verified() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let identity = user_db::get_sync_identity(&db, owner.id)
        .await
        .unwrap()
        .unwrap();
    let legacy = common::legacy_group(&db, &owner, "legacy").await;
    let writer = principal(&db, &owner).await;
    let claim = created(
        entity(
            EntityKind::Group,
            &legacy.unique_global_id,
            &random_hex_32(),
        ),
        &writer,
        1,
    );
    let result = causal_store::receive(&db, &owner, &identity, &writer, &wire(&claim)).await;
    assert!(
        matches!(result, Ok(ReceiveResult::AcceptedIdentityUnknown { .. })),
        "matching global id cannot prove identity of a local unknown-birth group"
    );
    assert_eq!(
        status(&db, &owner, &identity, &claim).await,
        IdentityStatus::Unknown
    );
    let birth: Option<String> =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(legacy.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(birth, None);
    let model_writer = principal(&db, &owner).await;
    let model = created(
        entity(
            EntityKind::Model,
            &legacy.unique_global_id,
            &random_hex_32(),
        ),
        &model_writer,
        1,
    );
    assert!(matches!(
        causal_store::receive(&db, &owner, &identity, &model_writer, &wire(&model)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    assert_eq!(
        status(&db, &owner, &identity, &model).await,
        IdentityStatus::Uncontested
    );
}

async fn check_remote_unknown_and_known_claims(unknown_first: bool) {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let identity = user_db::get_sync_identity(&db, owner.id)
        .await
        .unwrap()
        .unwrap();
    let legacy_writer = principal(&db, &owner).await;
    let known_writer = principal(&db, &owner).await;
    let global = random_hex_32();
    let unknown = CausalEnvelope {
        version: 2,
        entity: Entity {
            kind: EntityKind::Group,
            global_id: global.clone(),
            birth_id: None,
        },
        event: Dot {
            writer_id: legacy_writer.replica_id.clone(),
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![],
        action: Action::LegacyObserved {
            observation_digest: "old replica evidence".into(),
        },
    };
    let known = created(
        entity(EntityKind::Group, &global, &random_hex_32()),
        &known_writer,
        1,
    );
    let ordering = if unknown_first {
        [(&legacy_writer, &unknown), (&known_writer, &known)]
    } else {
        [(&known_writer, &known), (&legacy_writer, &unknown)]
    };
    for (writer, event) in ordering {
        let result = causal_store::receive(&db, &owner, &identity, writer, &wire(event)).await;
        assert!(result.is_ok(), "both observations must be retained");
    }
    assert_eq!(
        status(&db, &owner, &identity, &unknown).await,
        IdentityStatus::Unknown
    );
    assert_eq!(
        status(&db, &owner, &identity, &known).await,
        IdentityStatus::Unknown,
        "same global id with a legacy unknown birth must remain unresolved"
    );
    let other_id = user_db::add_user(&db, "other", "unknown-owner@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = user_db::get_sync_identity(&db, other.id)
        .await
        .unwrap()
        .unwrap();
    let other_writer = principal(&db, &other).await;
    let other_event = created(
        entity(EntityKind::Group, &global, &random_hex_32()),
        &other_writer,
        1,
    );
    assert!(matches!(
        causal_store::receive(
            &db,
            &other,
            &other_identity,
            &other_writer,
            &wire(&other_event)
        )
        .await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    assert_eq!(
        status(&db, &other, &other_identity, &other_event).await,
        IdentityStatus::Uncontested
    );
}

#[tokio::test]
async fn remote_unknown_and_known_birth_stay_unknown_in_either_arrival_order() {
    check_remote_unknown_and_known_claims(true).await;
    check_remote_unknown_and_known_claims(false).await;
}

#[tokio::test]
async fn known_model_claim_against_local_legacy_birth_stays_unknown() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let model_id = common::legacy_model(&db, &user, "legacy-model-claim").await;
    let global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(model_id)
            .fetch_one(&db)
            .await
            .unwrap();
    let writer = principal(&db, &user).await;
    let event = created(
        entity(EntityKind::Model, &global, &random_hex_32()),
        &writer,
        1,
    );
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &writer, &wire(&event)).await,
        Ok(ReceiveResult::AcceptedIdentityUnknown { .. })
    ));
    assert_eq!(
        status(&db, &user, &identity, &event).await,
        IdentityStatus::Unknown
    );
    let birth: Option<String> =
        sqlx::query_scalar("SELECT model_birth_id FROM models WHERE model_id = ?")
            .bind(model_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(birth, None);
}

#[tokio::test]
async fn purged_legacy_group_deletion_still_leaves_known_claim_identity_unknown() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = common::legacy_group(&db, &user, "legacy-purged").await;
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(group_db::purge_trashed_groups(&db, 30).await.unwrap(), 1);
    let writer = principal(&db, &user).await;
    let event = created(
        entity(EntityKind::Group, &group.unique_global_id, &random_hex_32()),
        &writer,
        1,
    );
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &writer, &wire(&event)).await,
        Ok(ReceiveResult::AcceptedIdentityUnknown { .. })
    ));
    assert_eq!(
        status(&db, &user, &identity, &event).await,
        IdentityStatus::Unknown
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn removed_empty_legacy_group_head_keeps_later_birth_claim_unknown() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let old = common::legacy_group(&db, &user, "old empty group").await;
    let known = group_db::add_empty_group(&db, &user, "known empty group", None)
        .await
        .unwrap();
    let known_birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(known.id)
            .fetch_one(&db)
            .await
            .unwrap();
    group_db::delete_dead_groups(&db).await.unwrap();
    let state: String = sqlx::query_scalar(
        "SELECT state FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&old.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(state, "absent");
    let records: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM group_deletion_records WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&old.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(records, 0);
    let writer = principal(&db, &user).await;
    let event = created(
        entity(EntityKind::Group, &old.unique_global_id, &random_hex_32()),
        &writer,
        1,
    );
    assert!(
        matches!(
            causal_store::receive(&db, &user, &identity, &writer, &wire(&event)).await,
            Ok(ReceiveResult::AcceptedIdentityUnknown { .. })
        ),
        "an absent legacy head is unresolved identity evidence"
    );
    assert_eq!(
        status(&db, &user, &identity, &event).await,
        IdentityStatus::Unknown
    );
    let known_writer = principal(&db, &user).await;
    let known_event = created(
        entity(EntityKind::Group, &known.unique_global_id, &known_birth),
        &known_writer,
        1,
    );
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &known_writer, &wire(&known_event)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    assert_eq!(
        status(&db, &user, &identity, &known_event).await,
        IdentityStatus::Uncontested
    );
    let model_writer = principal(&db, &user).await;
    let model_event = created(
        entity(EntityKind::Model, &old.unique_global_id, &random_hex_32()),
        &model_writer,
        1,
    );
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &model_writer, &wire(&model_event)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    let other_id = user_db::add_user(
        &db,
        "foreign",
        "legacy-head-other@example.test",
        &random_hex_32(),
    )
    .await
    .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = user_db::get_sync_identity(&db, other.id)
        .await
        .unwrap()
        .unwrap();
    let other_writer = principal(&db, &other).await;
    let other_event = created(
        entity(EntityKind::Group, &old.unique_global_id, &random_hex_32()),
        &other_writer,
        1,
    );
    assert!(matches!(
        causal_store::receive(
            &db,
            &other,
            &other_identity,
            &other_writer,
            &wire(&other_event)
        )
        .await,
        Ok(ReceiveResult::AcceptedReady)
    ));
}

#[tokio::test]
async fn purged_legacy_model_history_does_not_prove_a_new_birth_claim() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let old = common::legacy_model(&db, &user, "legacy-model-history").await;
    let old_global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(old)
            .fetch_one(&db)
            .await
            .unwrap();
    let blob = db::blob_db::add_blob(&db, "known-model-history", "stl", 1, None)
        .await
        .unwrap();
    let known = model_db::add_model(&db, &user, "known", blob, None, None)
        .await
        .unwrap();
    let (known_global, known_birth): (String, String) = sqlx::query_as(
        "SELECT model_unique_global_id, model_birth_id FROM models WHERE model_id = ?",
    )
    .bind(known)
    .fetch_one(&db)
    .await
    .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![old, known], None)
        .await
        .unwrap();
    model_db::trash_models(&db, &user, &[old, known])
        .await
        .unwrap();
    sqlx::query(
        "UPDATE models SET model_deleted = '2000-01-01T00:00:00Z' WHERE model_id IN (?, ?)",
    )
    .bind(old)
    .bind(known)
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(model_db::purge_trashed_models(&db, 30).await.unwrap(), 2);
    let retained: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM membership_causal_events WHERE user_id = ? AND model_global_id = ?",
    )
    .bind(user.id)
    .bind(&old_global)
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(retained > 0);
    let writer = principal(&db, &user).await;
    let event = created(
        entity(EntityKind::Model, &old_global, &random_hex_32()),
        &writer,
        1,
    );
    assert!(
        matches!(
            causal_store::receive(&db, &user, &identity, &writer, &wire(&event)).await,
            Ok(ReceiveResult::AcceptedIdentityUnknown { .. })
        ),
        "legacy model history has no portable birth proof"
    );
    assert_eq!(
        status(&db, &user, &identity, &event).await,
        IdentityStatus::Unknown
    );
    let known_writer = principal(&db, &user).await;
    let known_event = created(
        entity(EntityKind::Model, &known_global, &known_birth),
        &known_writer,
        1,
    );
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &known_writer, &wire(&known_event)).await,
        Ok(ReceiveResult::AcceptedReady)
    ));
    assert_eq!(
        status(&db, &user, &identity, &known_event).await,
        IdentityStatus::Uncontested
    );
}

#[tokio::test]
async fn rekeyed_local_creation_is_unknown_not_remote_conflict_for_group_and_model() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &user, "local", None)
        .await
        .unwrap();
    let group_birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let group_new = random_hex_32();
    group_db::edit_group(&db, &user, group.id, "local", None, Some(&group_new))
        .await
        .unwrap();
    let blob = db::blob_db::add_blob(&db, "rekey-status", "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "model", blob, None, None)
        .await
        .unwrap();
    let model_birth: String =
        sqlx::query_scalar("SELECT model_birth_id FROM models WHERE model_id = ?")
            .bind(model)
            .fetch_one(&db)
            .await
            .unwrap();
    let model_new = random_hex_32();
    model_db::edit_model(
        &db,
        &user,
        model,
        "model",
        None,
        None,
        db::model::ModelFlags::empty(),
        None,
        Some(&model_new),
    )
    .await
    .unwrap();
    for (kind, global, birth) in [
        (EntityKind::Group, group_new, group_birth),
        (EntityKind::Model, model_new, model_birth),
    ] {
        let writer = principal(&db, &user).await;
        let claim = created(entity(kind.clone(), &global, &birth), &writer, 1);
        let result = causal_store::receive(&db, &user, &identity, &writer, &wire(&claim))
            .await
            .unwrap();
        assert!(
            matches!(result, ReceiveResult::AcceptedIdentityUnknown { .. }),
            "old local creation is incomplete history, not a competing remote identity"
        );
        assert_eq!(
            status(&db, &user, &identity, &claim).await,
            IdentityStatus::Unknown
        );
        let competitor = created(entity(kind, &global, &random_hex_32()), &writer, 2);
        let result = causal_store::receive(&db, &user, &identity, &writer, &wire(&competitor))
            .await
            .unwrap();
        assert!(
            matches!(result, ReceiveResult::AcceptedIdentityConflict { .. }),
            "real incompatible remote claims remain conflicts"
        );
        assert_eq!(
            status(&db, &user, &identity, &claim).await,
            IdentityStatus::Conflict
        );
    }
}
