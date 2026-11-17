//! Recreating a purged group must not silently lose its portable identity.
mod common;
use db::{
    group_db::{self, GroupLifecycleChange, GroupPrecondition},
    model::user::User,
    random_hex_32,
};

#[tokio::test]
async fn keep_after_purge_fails_closed_until_birth_rehydration_exists() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let deletion = group_db::get_group_sync_snapshot(&db, &user)
        .await
        .unwrap()
        .deletions
        .into_iter()
        .find(|d| d.group_global_id == group.unique_global_id)
        .unwrap()
        .operation_id;
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(&db)
    .await
    .unwrap();
    group_db::purge_trashed_groups(&db, 30).await.unwrap();
    let snapshot = group_db::get_group_sync_snapshot(&db, &user).await.unwrap();
    let head = snapshot
        .heads
        .iter()
        .find(|h| h.group_global_id == group.unique_global_id)
        .unwrap();
    let condition = GroupPrecondition {
        group_global_id: group.unique_global_id.clone(),
        expected_generation: head.generation,
        expected_head: head.head,
        expected_state: head.state.clone(),
        expected_revision: None,
        expected_deleted_at: None,
    };
    let action = GroupLifecycleChange::Keep {
        group_global_id: group.unique_global_id.clone(),
        deletion_operation_id: deletion,
        expected_birth_id: None,
        resource: group_db::KeepResource::None,
        name: "folder".into(),
    };
    let result = group_db::apply_group_sync_batch(
        &db,
        &user,
        &random_hex_32(),
        &[],
        &[condition],
        &[action],
        30,
    )
    .await;
    assert!(
        result.is_err(),
        "recreation must fail closed until it preserves the original birth id"
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?",
    )
    .bind(user.id)
    .bind(&group.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(count, 0);
}
