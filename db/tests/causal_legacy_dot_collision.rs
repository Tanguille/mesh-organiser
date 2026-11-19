//! A dot received through v2 cannot be reused by the legacy rename inbox.
use db::{causal_store, group_history, random_hex_32};

mod common;

#[tokio::test]
async fn v2_receipt_prevents_legacy_rename_reusing_its_dot() {
    let (_dir, db) = common::test_db().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = db::group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let writer = random_hex_32();
    sqlx::query("INSERT INTO causal_replica_owners (user_id, replica_id) VALUES (?, ?)")
        .bind(user.id)
        .bind(&writer)
        .execute(&db)
        .await
        .unwrap();
    let principal = group_history::ReplicaPrincipal {
        owner_user_id: user.id,
        replica_id: writer.clone(),
    };
    let event = db::causal_envelope::CausalEnvelope {
        version: 2,
        entity: db::causal_envelope::Entity {
            kind: db::causal_envelope::EntityKind::Group,
            global_id: group.unique_global_id.clone(),
            birth_id: Some(birth),
        },
        event: db::causal_envelope::Dot {
            writer_id: writer.clone(),
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![],
        action: db::causal_envelope::Action::GroupDeleted {
            deletion_operation_id: random_hex_32(),
        },
    };
    let wire = db::causal_envelope::canonical_causal_envelope(&event).unwrap();
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &principal, &wire).await,
        Ok(causal_store::ReceiveResult::AcceptedReady)
    ));
    let forged = group_history::GroupRenameEvent {
        id: group_history::EventId {
            replica_id: writer,
            counter: 1,
        },
        group_global_id: group.unique_global_id.clone(),
        parents: vec![],
        name: "different action".into(),
    };
    assert!(
        group_history::ingest_group_rename(&db, &principal, &forged)
            .await
            .is_err(),
        "legacy inbox must not reuse a v2 receipt dot"
    );
    assert_eq!(
        group_history::get_received_group_renames(&db, &user, &group.unique_global_id)
            .await
            .unwrap(),
        []
    );
    assert!(
        causal_store::get_received(&db, &user, &identity, &event.event)
            .await
            .unwrap()
            .is_some()
    );
}
