//! Purging a legacy model must not erase the fact that its portable birth is unknown.
mod common;
use db::{
    causal_envelope::{self, Action, CausalEnvelope, Dot, Entity, EntityKind},
    causal_store::{self, IdentityStatus, ReceiveResult},
    group_history::ReplicaPrincipal,
    model::{ModelFlags, user::User},
    model_db, random_hex_32, user_db,
};

async fn send_claim(
    db: &db::db_context::DbContext,
    user: &User,
    global: &str,
    kind: EntityKind,
) -> (ReceiveResult, CausalEnvelope) {
    let identity = user_db::get_sync_identity(db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = random_hex_32();
    sqlx::query("INSERT INTO causal_replica_owners (user_id, replica_id) VALUES (?, ?)")
        .bind(user.id)
        .bind(&writer)
        .execute(db)
        .await
        .unwrap();
    let action = match kind {
        EntityKind::Group => Action::GroupCreated {
            name: "claimed".into(),
        },
        EntityKind::Model => Action::ModelCreated,
    };
    let event = CausalEnvelope {
        version: 2,
        entity: Entity {
            kind,
            global_id: global.into(),
            birth_id: Some(random_hex_32()),
        },
        event: Dot {
            writer_id: writer.clone(),
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![],
        action,
    };
    let principal = ReplicaPrincipal {
        owner_user_id: user.id,
        replica_id: writer,
    };
    let wire = causal_envelope::canonical_causal_envelope(&event).unwrap();
    let result = causal_store::receive(db, user, &identity, &principal, &wire)
        .await
        .unwrap();
    (result, event)
}

#[tokio::test]
async fn legacy_model_rekey_retains_old_unknown_identity_only_for_its_owner_and_kind() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let id = common::legacy_model(&db, &user, "legacy-rekey").await;
    let old: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    model_db::edit_model(
        &db,
        &user,
        id,
        "legacy-rekey",
        None,
        None,
        ModelFlags::empty(),
        None,
        Some(&old),
    )
    .await
    .unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM unknown_model_identity_tombstones WHERE user_id = ?",
    )
    .bind(user.id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(count, 0, "a no-op rekey must not create evidence");
    let new = random_hex_32();
    model_db::edit_model(
        &db,
        &user,
        id,
        "legacy-rekey",
        None,
        None,
        ModelFlags::empty(),
        None,
        Some(&new),
    )
    .await
    .unwrap();
    let (result, _) = send_claim(&db, &user, &old, EntityKind::Model).await;
    assert!(
        matches!(result, ReceiveResult::AcceptedIdentityUnknown { .. }),
        "rekey cannot erase the old unknown identity"
    );
    let (group_result, _) = send_claim(&db, &user, &old, EntityKind::Group).await;
    assert!(matches!(group_result, ReceiveResult::AcceptedReady));
    let other_id = user_db::add_user(
        &db,
        "other-rekey",
        "legacy-rekey-other@example.test",
        &random_hex_32(),
    )
    .await
    .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let (foreign_result, _) = send_claim(&db, &other, &old, EntityKind::Model).await;
    assert!(matches!(foreign_result, ReceiveResult::AcceptedReady));
}

#[tokio::test]
async fn failed_unknown_marker_insert_rolls_back_legacy_model_rekey() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let id = common::legacy_model(&db, &user, "legacy-rekey-failure").await;
    let old: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    sqlx::query("CREATE TRIGGER reject_rekey_marker BEFORE INSERT ON unknown_model_identity_tombstones BEGIN SELECT RAISE(ABORT, 'injected failure'); END").execute(&db).await.unwrap();
    assert!(
        model_db::edit_model(
            &db,
            &user,
            id,
            "changed",
            None,
            None,
            ModelFlags::empty(),
            None,
            Some(&random_hex_32())
        )
        .await
        .is_err()
    );
    let actual: (String, String) =
        sqlx::query_as("SELECT model_unique_global_id, model_name FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(actual, (old, "legacy-rekey-failure".to_owned()));
}

#[tokio::test]
async fn never_moved_legacy_model_retains_unknown_identity_after_purge() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let id = common::legacy_model(&db, &user, "unmoved-legacy").await;
    let global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    let events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM membership_causal_events WHERE user_id = ? AND model_global_id = ?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(events, 0);
    model_db::trash_models(&db, &user, &[id]).await.unwrap();
    sqlx::query("UPDATE models SET model_deleted = '2000-01-01T00:00:00Z' WHERE model_id = ?")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(model_db::purge_trashed_models(&db, 30).await.unwrap(), 1);
    let (result, event) = send_claim(&db, &user, &global, EntityKind::Model).await;
    assert!(
        matches!(result, ReceiveResult::AcceptedIdentityUnknown { .. }),
        "a purged legacy model cannot acquire a verified birth by global id alone"
    );
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let stored = causal_store::get_received(&db, &user, &identity, &event.event)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.identity_status, IdentityStatus::Unknown);
    let (group_result, _) = send_claim(&db, &user, &global, EntityKind::Group).await;
    assert!(matches!(group_result, ReceiveResult::AcceptedReady));
    let other_id = user_db::add_user(
        &db,
        "other",
        "legacy-purge-other@example.test",
        &random_hex_32(),
    )
    .await
    .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let (foreign_result, _) = send_claim(&db, &other, &global, EntityKind::Model).await;
    assert!(matches!(foreign_result, ReceiveResult::AcceptedReady));
}

#[tokio::test]
async fn failed_unknown_marker_insert_rolls_back_model_purge() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let id = common::legacy_model(&db, &user, "failed-marker").await;
    model_db::trash_models(&db, &user, &[id]).await.unwrap();
    sqlx::query("UPDATE models SET model_deleted = '2000-01-01T00:00:00Z' WHERE model_id = ?")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER reject_unknown_model_marker BEFORE INSERT ON unknown_model_identity_tombstones BEGIN SELECT RAISE(ABORT, 'injected failure'); END").execute(&db).await.unwrap();
    assert!(model_db::purge_trashed_models(&db, 30).await.is_err());
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM models WHERE model_id = ? AND model_user_id = ?)",
    )
    .bind(id)
    .bind(user.id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(
        exists,
        "model purge must roll back if its unknown marker cannot be recorded"
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM unknown_model_identity_tombstones WHERE user_id = ?",
    )
    .bind(user.id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn deleting_account_does_not_leak_unknown_model_markers_to_reused_user_id() {
    let (_dir, db) = common::test_db().await;
    let id = user_db::add_user(
        &db,
        "old",
        "legacy-delete-old@example.test",
        &random_hex_32(),
    )
    .await
    .unwrap();
    let old = user_db::get_user_by_id(&db, id).await.unwrap().unwrap();
    let model = common::legacy_model(&db, &old, "legacy-account-model").await;
    let global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(model)
            .fetch_one(&db)
            .await
            .unwrap();
    sqlx::query("DELETE FROM users WHERE user_id = ?")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (user_id, user_name, user_email, user_password_hash, user_created_at) VALUES (?, 'replacement', 'legacy-delete-new@example.test', 'unused', '2020-01-01T00:00:00Z')").bind(id).execute(&db).await.unwrap();
    let replacement = user_db::get_user_by_id(&db, id).await.unwrap().unwrap();
    let (result, _) = send_claim(&db, &replacement, &global, EntityKind::Model).await;
    assert!(
        matches!(result, ReceiveResult::AcceptedReady),
        "old account tombstones must not contaminate the new account"
    );
}
