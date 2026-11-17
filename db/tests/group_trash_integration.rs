//! Group deletion retains an owner-scoped recovery snapshot while ungrouping its models.

use db::{
    db_context::DbContext,
    group_db::{self, GroupFilterOptions},
    model::user::User,
    model_db, random_hex_32, user_db,
};

mod common;

async fn add_model(db: &DbContext, user: &User, name: &str) -> i64 {
    let blob_id = db::blob_db::add_blob(db, name, "stl", 1, None)
        .await
        .unwrap();
    model_db::add_model(db, user, name, blob_id, None, None)
        .await
        .unwrap()
}

async fn model_group(db: &DbContext, model_id: i64) -> Option<i64> {
    sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
        .bind(model_id)
        .fetch_one(db)
        .await
        .unwrap()
}

async fn mark_deleted_before_retention(db: &DbContext, group_id: i64) {
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group_id)
    .execute(db)
    .await
    .unwrap();
}

#[tokio::test]
async fn deleted_group_is_retained_for_recovery_but_hidden_and_its_models_are_ungrouped() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.com", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let own = group_db::add_empty_group(&db, &user, "recoverable folder", None)
        .await
        .unwrap();
    let foreign = group_db::add_empty_group(&db, &other, "other folder", None)
        .await
        .unwrap();
    let live = add_model(&db, &user, "live-part").await;
    let trashed = add_model(&db, &user, "trashed-part").await;
    group_db::set_group_id_on_models(&db, &user, Some(own.id), vec![live, trashed], None)
        .await
        .unwrap();
    model_db::trash_models(&db, &user, &[trashed])
        .await
        .unwrap();

    group_db::delete_group(&db, &user, own.id).await.unwrap();

    let retained_name: Option<String> = sqlx::query_scalar(
        "SELECT group_name FROM models_group WHERE group_id = ? AND group_user_id = ?",
    )
    .bind(own.id)
    .bind(user.id)
    .fetch_optional(&db)
    .await
    .unwrap();
    assert_eq!(retained_name.as_deref(), Some("recoverable folder"));
    let visible = group_db::get_groups(&db, &user, GroupFilterOptions::default())
        .await
        .unwrap();
    assert!(
        visible.items.is_empty(),
        "trashed groups must not appear in normal listings"
    );
    for model_id in [live, trashed] {
        let group_id: Option<i64> =
            sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
                .bind(model_id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(group_id, None, "model {model_id} must remain ungrouped");
    }
    let foreign_groups = group_db::get_groups(&db, &other, GroupFilterOptions::default())
        .await
        .unwrap();
    assert_eq!(foreign_groups.items.len(), 1);
    assert_eq!(foreign_groups.items[0].meta.id, foreign.id);
}

#[tokio::test]
async fn failed_model_ungroup_rolls_back_group_deletion_and_preserves_all_memberships() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "atomic folder", None)
        .await
        .unwrap();
    let first = add_model(&db, &user, "first-part").await;
    let second = add_model(&db, &user, "second-part").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![first, second], None)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_ungroup BEFORE UPDATE OF model_group_id ON models
         WHEN NEW.model_group_id IS NULL
         BEGIN SELECT RAISE(ABORT, 'injected ungroup failure'); END",
    )
    .execute(&db)
    .await
    .unwrap();

    let result = group_db::delete_group(&db, &user, group.id).await;

    assert!(
        result.is_err(),
        "the injected failure must reach the caller"
    );
    let groups = group_db::get_groups(&db, &user, GroupFilterOptions::default())
        .await
        .unwrap();
    assert_eq!(groups.items.len(), 1);
    assert_eq!(groups.items[0].meta.id, group.id);
    let mut remaining = groups.items[0]
        .models
        .iter()
        .map(|model| model.id)
        .collect::<Vec<_>>();
    remaining.sort_unstable();
    assert_eq!(remaining, vec![first, second]);
    let operation_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM group_deletion_records WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&group.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
        operation_count, 0,
        "a failed deletion must not leave a journal entry"
    );
    let saved_members: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM group_recovery_members WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(
        saved_members, 0,
        "a failed deletion must not leave a recovery snapshot"
    );
}

#[tokio::test]
async fn restore_reattaches_only_former_members_that_are_still_ungrouped() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let original = group_db::add_empty_group(&db, &user, "original", None)
        .await
        .unwrap();
    let destination = group_db::add_empty_group(&db, &user, "destination", None)
        .await
        .unwrap();
    let live = add_model(&db, &user, "restore-live").await;
    let moved = add_model(&db, &user, "restore-moved").await;
    let trashed = add_model(&db, &user, "restore-trashed").await;
    group_db::set_group_id_on_models(
        &db,
        &user,
        Some(original.id),
        vec![live, moved, trashed],
        None,
    )
    .await
    .unwrap();
    model_db::trash_models(&db, &user, &[trashed])
        .await
        .unwrap();
    group_db::delete_group(&db, &user, original.id)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(destination.id), vec![moved], None)
        .await
        .unwrap();

    let deleted = group_db::get_trashed_groups(&db, &user, 30).await.unwrap();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].id, original.id);
    assert_eq!(deleted[0].name, "original");
    let restore = common::group_restore_request(&db, &user, original.id).await;
    group_db::restore_groups(&db, &user, &[restore], 30)
        .await
        .unwrap();

    assert_eq!(model_group(&db, live).await, Some(original.id));
    assert_eq!(model_group(&db, trashed).await, Some(original.id));
    assert_eq!(model_group(&db, moved).await, Some(destination.id));
    assert!(
        group_db::get_trashed_groups(&db, &user, 30)
            .await
            .unwrap()
            .is_empty()
    );
    let trashed_at: Option<String> =
        sqlx::query_scalar("SELECT model_deleted FROM models WHERE model_id = ?")
            .bind(trashed)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(
        trashed_at.is_some(),
        "restoring a group must not restore a trashed model"
    );
}

#[tokio::test]
async fn restore_rejects_unknown_foreign_or_expired_groups_without_partial_changes() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.com", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let own = group_db::add_empty_group(&db, &user, "own", None)
        .await
        .unwrap();
    let expired = group_db::add_empty_group(&db, &user, "expired", None)
        .await
        .unwrap();
    let foreign = group_db::add_empty_group(&db, &other, "foreign", None)
        .await
        .unwrap();
    let member = add_model(&db, &user, "atomic-restore-member").await;
    group_db::set_group_id_on_models(&db, &user, Some(own.id), vec![member], None)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, own.id).await.unwrap();
    group_db::delete_group(&db, &user, expired.id)
        .await
        .unwrap();
    group_db::delete_group(&db, &other, foreign.id)
        .await
        .unwrap();
    let own_request = common::group_restore_request(&db, &user, own.id).await;
    let expired_request = common::group_restore_request(&db, &user, expired.id).await;
    let foreign_request = common::group_restore_request(&db, &other, foreign.id).await;
    mark_deleted_before_retention(&db, expired.id).await;

    let mut unknown = own_request.clone();
    unknown.id = -1;
    for refused in [unknown, foreign_request, expired_request] {
        let refused_id = refused.id;
        let result =
            group_db::restore_groups(&db, &user, &[own_request.clone(), refused], 30).await;
        assert!(
            result.is_err(),
            "group {refused_id} must cause the whole restore to fail"
        );
        assert_eq!(model_group(&db, member).await, None);
        let own_deleted_at: Option<String> =
            sqlx::query_scalar("SELECT group_deleted_at FROM models_group WHERE group_id = ?")
                .bind(own.id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert!(
            own_deleted_at.is_some(),
            "valid group must stay trashed after rejection"
        );
    }
    let other_deleted = group_db::get_trashed_groups(&db, &other, 30).await.unwrap();
    assert_eq!(other_deleted.len(), 1);
    assert_eq!(other_deleted[0].id, foreign.id);
}

#[tokio::test]
async fn repeated_deletion_and_restore_are_safe_to_retry() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "retry", None)
        .await
        .unwrap();
    let member = add_model(&db, &user, "retry-member").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![member], None)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let first_deleted: Option<String> =
        sqlx::query_scalar("SELECT group_deleted_at FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let first_ops: Vec<String> = sqlx::query_scalar(
        "SELECT operation_id FROM group_deletion_records WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&group.unique_global_id)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(first_ops.len(), 1);

    group_db::delete_group(&db, &user, group.id).await.unwrap();

    let second_deleted: Option<String> =
        sqlx::query_scalar("SELECT group_deleted_at FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let second_ops: Vec<String> = sqlx::query_scalar(
        "SELECT operation_id FROM group_deletion_records WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&group.unique_global_id)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(first_deleted, second_deleted);
    assert_eq!(first_ops, second_ops);
    let restore = common::group_restore_request(&db, &user, group.id).await;
    group_db::restore_groups(&db, &user, std::slice::from_ref(&restore), 30)
        .await
        .unwrap();
    group_db::restore_groups(&db, &user, &[restore], 30)
        .await
        .unwrap();
    assert_eq!(model_group(&db, member).await, Some(group.id));
}

#[tokio::test]
async fn expiry_removes_recovery_data_but_preserves_the_durable_deletion_record() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "expired", None)
        .await
        .unwrap();
    let member = add_model(&db, &user, "expired-member").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![member], None)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let restore = common::group_restore_request(&db, &user, group.id).await;
    mark_deleted_before_retention(&db, group.id).await;

    let purged = group_db::purge_trashed_groups(&db, 30).await.unwrap();

    assert_eq!(purged, 1);
    assert!(
        group_db::get_trashed_groups(&db, &user, 30)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        group_db::restore_groups(&db, &user, &[restore], 30)
            .await
            .is_err()
    );
    assert_eq!(model_group(&db, member).await, None);
    let retained: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM group_deletion_records WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&group.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
        retained, 1,
        "offline devices still need the deletion record"
    );
    let snapshots: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM group_recovery_members WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(snapshots, 0);
}

#[tokio::test]
async fn assigning_models_to_a_trashed_group_is_rejected_without_moving_them() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let source = group_db::add_empty_group(&db, &user, "source", None)
        .await
        .unwrap();
    let deleted = group_db::add_empty_group(&db, &user, "deleted", None)
        .await
        .unwrap();
    let member = add_model(&db, &user, "preserve-source-member").await;
    group_db::set_group_id_on_models(&db, &user, Some(source.id), vec![member], None)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, deleted.id)
        .await
        .unwrap();

    let result =
        group_db::set_group_id_on_models(&db, &user, Some(deleted.id), vec![member], None).await;

    assert!(result.is_err(), "a trashed group cannot receive models");
    assert_eq!(model_group(&db, member).await, Some(source.id));
}

#[tokio::test]
async fn editing_a_trashed_group_cannot_change_its_recoverable_metadata() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "original name", None)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();

    let _ = group_db::edit_group(&db, &user, group.id, "changed name", None, None).await;

    let metadata: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT group_name, group_deleted_at FROM models_group WHERE group_id = ? AND group_user_id = ?",
    )
    .bind(group.id)
    .bind(user.id)
    .fetch_optional(&db)
    .await
    .unwrap();
    let (name, deleted_at) = metadata.expect("the group remains available for recovery");
    assert_eq!(name, "original name");
    assert!(deleted_at.is_some(), "the group must remain in Trash");
}
