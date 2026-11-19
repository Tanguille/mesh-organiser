//! Retired local writers cannot be accepted as remote event authors.
use db::group_history::{self, EventId, GroupRenameEvent, ReplicaPrincipal};

mod common;

async fn check_retired_writer(reuse_creation_dot: bool) {
    let (dir, db) = common::test_db().await;
    let user = db::model::user::User::default();
    let group = db::group_db::add_empty_group(&db, &user, "local", None)
        .await
        .unwrap();
    let birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let creation = db::causal_envelope::get_local_creation_record(
        &db,
        &user,
        db::causal_envelope::EntityKind::Group,
        &birth,
    )
    .await
    .unwrap()
    .unwrap();
    let old_writer = creation.envelope.event.writer_id;
    let old_counter: i64 = creation.envelope.event.counter.parse().unwrap();
    db.close().await;
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    let current_writer: String =
        sqlx::query_scalar("SELECT replica_id FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_ne!(
        old_writer, current_writer,
        "fixture must retire the original writer"
    );
    sqlx::query("INSERT INTO causal_replica_owners (user_id, replica_id) VALUES (?, ?)")
        .bind(user.id)
        .bind(&old_writer)
        .execute(&db)
        .await
        .unwrap();
    let principal = ReplicaPrincipal {
        owner_user_id: user.id,
        replica_id: old_writer.clone(),
    };
    let event = GroupRenameEvent {
        id: EventId {
            replica_id: old_writer,
            counter: if reuse_creation_dot {
                old_counter
            } else {
                old_counter + 100
            },
        },
        group_global_id: group.unique_global_id.clone(),
        parents: vec![],
        name: "forged remote".into(),
    };
    assert!(
        group_history::ingest_group_rename(&db, &principal, &event)
            .await
            .is_err(),
        "retired local writer must not be accepted as remote"
    );
    let remote = db::random_hex_32();
    sqlx::query("INSERT INTO causal_replica_owners (user_id, replica_id) VALUES (?, ?)")
        .bind(user.id)
        .bind(&remote)
        .execute(&db)
        .await
        .unwrap();
    let principal = ReplicaPrincipal {
        owner_user_id: user.id,
        replica_id: remote.clone(),
    };
    let event = GroupRenameEvent {
        id: EventId {
            replica_id: remote,
            counter: 1,
        },
        group_global_id: group.unique_global_id.clone(),
        parents: vec![],
        name: "legitimate".into(),
    };
    assert!(matches!(
        group_history::ingest_group_rename(&db, &principal, &event).await,
        Ok(group_history::IngestResult::AcceptedReady)
    ));
    let received = group_history::get_received_group_renames(&db, &user, &group.unique_global_id)
        .await
        .unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].event.name, "legitimate");
}

#[tokio::test]
async fn retired_writer_cannot_reuse_a_local_creation_dot() {
    check_retired_writer(true).await;
}

#[tokio::test]
async fn retired_writer_cannot_issue_a_previously_unused_dot() {
    check_retired_writer(false).await;
}
