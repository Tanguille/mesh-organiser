//! Group lifecycle and model assignment sync must commit as one guarded operation.
use db::{
    db_context::DbContext,
    group_db::{
        self, AssignmentBatchResult, AssignmentChange, GroupLifecycleChange, GroupPrecondition,
    },
    model::user::User,
    model_db, random_hex_32, user_db,
};

mod common;

async fn model(db: &DbContext, user: &User, name: &str) -> (i64, String) {
    let blob = db::blob_db::add_blob(db, name, "stl", 1, None)
        .await
        .unwrap();
    let id = model_db::add_model(db, user, name, blob, None, None)
        .await
        .unwrap();
    let global = sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap();
    (id, global)
}
async fn change(
    db: &DbContext,
    user: &User,
    global: &str,
    target: Option<&str>,
) -> AssignmentChange {
    let snapshot = group_db::get_group_sync_snapshot(db, user).await.unwrap();
    let item = snapshot
        .assignments
        .iter()
        .find(|assignment| assignment.model_global_id == global)
        .unwrap();
    AssignmentChange {
        model_global_id: global.into(),
        expected_incarnation: item.incarnation.clone(),
        expected_group_global_id: item.group_global_id.clone(),
        expected_revision: item.revision,
        expected_lifecycle_revision: item.lifecycle_revision,
        expected_deleted_at: item.deleted_at.clone(),
        target_group_global_id: target.map(str::to_owned),
    }
}
async fn condition(db: &DbContext, user: &User, global: &str) -> GroupPrecondition {
    let snapshot = group_db::get_group_sync_snapshot(db, user).await.unwrap();
    let group_head = snapshot
        .heads
        .iter()
        .find(|head| head.group_global_id == global)
        .unwrap();
    let group = snapshot
        .groups
        .iter()
        .find(|group| group.global_id == global);
    GroupPrecondition {
        group_global_id: global.into(),
        expected_generation: group_head.generation,
        expected_head: group_head.head,
        expected_state: group_head.state.clone(),
        expected_revision: group.map(|group| group.revision),
        expected_deleted_at: group.and_then(|group| group.deleted_at.clone()),
    }
}
async fn assignments(db: &DbContext, user: &User) -> Vec<(String, Option<String>)> {
    group_db::get_group_sync_snapshot(db, user)
        .await
        .unwrap()
        .assignments
        .into_iter()
        .map(|assignment| (assignment.model_global_id, assignment.group_global_id))
        .collect()
}
fn delete(global: &str, deletion_id: &str) -> GroupLifecycleChange {
    GroupLifecycleChange::Delete {
        group_global_id: global.into(),
        deletion_operation_id: deletion_id.into(),
    }
}
fn restore(global: &str, deletion_id: &str) -> GroupLifecycleChange {
    GroupLifecycleChange::Restore {
        group_global_id: global.into(),
        deletion_operation_id: deletion_id.into(),
    }
}
fn keep(global: &str, deletion_id: &str, birth: Option<&str>) -> GroupLifecycleChange {
    GroupLifecycleChange::Keep {
        group_global_id: global.into(),
        deletion_operation_id: deletion_id.into(),
        expected_birth_id: birth.map(str::to_owned),
        resource: group_db::KeepResource::None,
        name: "kept".into(),
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn delete_and_explicit_restore_are_atomic_and_replay_safe() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let global = &group.unique_global_id;
    let (live, live_global) = model(&db, &user, "live").await;
    let (trashed, trashed_global) = model(&db, &user, "trash").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![live, trashed], None)
        .await
        .unwrap();
    model_db::trash_models(&db, &user, &[trashed])
        .await
        .unwrap();
    let one = change(&db, &user, &live_global, None).await;
    let two = change(&db, &user, &trashed_global, None).await;
    let before = condition(&db, &user, global).await;
    let deletion_id = random_hex_32();
    let action = delete(global, &deletion_id);
    let op = random_hex_32();
    let result = group_db::apply_group_sync_batch(
        &db,
        &user,
        &op,
        &[one.clone(), two.clone()],
        std::slice::from_ref(&before),
        std::slice::from_ref(&action),
        30,
    )
    .await;
    assert!(matches!(result, Ok(AssignmentBatchResult::Applied)));
    let after = assignments(&db, &user).await;
    assert!(after.iter().all(|(_, group)| group.is_none()));
    let snapshot = group_db::get_group_sync_snapshot(&db, &user).await.unwrap();
    assert!(
        snapshot
            .groups
            .iter()
            .find(|group| &group.global_id == global)
            .unwrap()
            .deleted_at
            .is_some()
    );
    assert!(
        snapshot
            .deletions
            .iter()
            .any(|deletion| deletion.operation_id == deletion_id && deletion.restored_at.is_none())
    );
    assert!(matches!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &op,
            &[two.clone(), one.clone()],
            std::slice::from_ref(&before),
            &[action],
            30
        )
        .await,
        Ok(AssignmentBatchResult::AlreadyApplied)
    ));
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &op,
            &[one, two],
            &[before],
            &[delete(global, &random_hex_32())],
            30
        )
        .await
        .is_err()
    );
    let reassign = change(&db, &user, &live_global, Some(global)).await;
    let trashed_head = condition(&db, &user, global).await;
    assert!(matches!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[reassign],
            &[trashed_head],
            &[restore(global, &deletion_id)],
            30
        )
        .await,
        Ok(AssignmentBatchResult::Applied)
    ));
    let after = assignments(&db, &user).await;
    assert_eq!(
        after
            .iter()
            .find(|(id, _)| id == &live_global)
            .unwrap()
            .1
            .as_deref(),
        Some(global.as_str())
    );
    assert_eq!(
        after
            .iter()
            .find(|(id, _)| id == &trashed_global)
            .unwrap()
            .1,
        None,
        "restore must not implicitly reattach unchecked members"
    );
    let recovery: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM group_recovery_members WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(recovery, 0);
}

#[tokio::test]
async fn keep_recreates_purged_group_only_for_matching_deletion() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = common::legacy_group(&db, &user, "folder").await;
    let global = &group.unique_global_id;
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let snap = group_db::get_group_sync_snapshot(&db, &user).await.unwrap();
    let deletion = snap
        .deletions
        .iter()
        .find(|deletion| &deletion.group_global_id == global)
        .unwrap()
        .operation_id
        .clone();
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(&db)
    .await
    .unwrap();
    group_db::purge_trashed_groups(&db, 30).await.unwrap();
    let head = condition(&db, &user, global).await;
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[],
            std::slice::from_ref(&head),
            &[restore(global, &deletion)],
            30
        )
        .await
        .is_err()
    );
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[],
            std::slice::from_ref(&head),
            &[keep(global, &random_hex_32(), None)],
            30
        )
        .await
        .is_err()
    );
    let (_, model_global) = model(&db, &user, "part").await;
    let assignment = change(&db, &user, &model_global, Some(global)).await;
    assert!(matches!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[assignment],
            std::slice::from_ref(&head),
            &[keep(global, &deletion, None)],
            30
        )
        .await,
        Ok(AssignmentBatchResult::Applied)
    ));
    let after = condition(&db, &user, global).await;
    assert_eq!(after.expected_state, "active");
    assert!(after.expected_generation > head.expected_generation);
    assert_eq!(
        assignments(&db, &user).await[0].1.as_deref(),
        Some(global.as_str())
    );
}

#[tokio::test]
async fn delete_requires_all_live_and_trashed_members_with_current_preconditions() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let global = &group.unique_global_id;
    let (live, live_global) = model(&db, &user, "live").await;
    let (trash, trash_global) = model(&db, &user, "trash").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![live, trash], None)
        .await
        .unwrap();
    model_db::trash_models(&db, &user, &[trash]).await.unwrap();
    let one = change(&db, &user, &live_global, None).await;
    let two = change(&db, &user, &trash_global, None).await;
    let before = condition(&db, &user, global).await;
    let operation = delete(global, &random_hex_32());
    let old = assignments(&db, &user).await;
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            std::slice::from_ref(&one),
            std::slice::from_ref(&before),
            std::slice::from_ref(&operation),
            30
        )
        .await
        .is_err()
    );
    let mut wrong = two.clone();
    wrong.expected_incarnation = random_hex_32();
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[one.clone(), wrong],
            std::slice::from_ref(&before),
            std::slice::from_ref(&operation),
            30
        )
        .await
        .is_err()
    );
    let mut stale = before.clone();
    stale.expected_head -= 1;
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[one, two],
            &[stale],
            &[operation],
            30
        )
        .await
        .is_err()
    );
    assert_eq!(assignments(&db, &user).await, old);
    assert_eq!(
        condition(&db, &user, global).await.expected_head,
        before.expected_head
    );
}

#[tokio::test]
async fn lifecycle_and_assignments_roll_back_if_journal_write_fails() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let (id, global) = model(&db, &user, "part").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![id], None)
        .await
        .unwrap();
    let ch = change(&db, &user, &global, None).await;
    let before = condition(&db, &user, &group.unique_global_id).await;
    let old = assignments(&db, &user).await;
    sqlx::query("CREATE TRIGGER reject_batch_journal BEFORE INSERT ON assignment_batch_journal BEGIN SELECT RAISE(ABORT, 'journal failure'); END").execute(&db).await.unwrap();
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[ch],
            std::slice::from_ref(&before),
            &[delete(&group.unique_global_id, &random_hex_32())],
            30
        )
        .await
        .is_err()
    );
    assert_eq!(assignments(&db, &user).await, old);
    let after = condition(&db, &user, &group.unique_global_id).await;
    assert_eq!(after.expected_head, before.expected_head);
    assert_eq!(after.expected_state, "active");
    assert!(
        group_db::get_group_sync_snapshot(&db, &user)
            .await
            .unwrap()
            .deletions
            .is_empty()
    );
}

#[tokio::test]
async fn foreign_group_and_unknown_group_fail_with_same_error() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &other, "private", None)
        .await
        .unwrap();
    let foreign = condition(&db, &other, &group.unique_global_id).await;
    let mut unknown = foreign.clone();
    unknown.group_global_id = random_hex_32();
    let foreign_error = group_db::apply_group_sync_batch(
        &db,
        &user,
        &random_hex_32(),
        &[],
        &[foreign],
        &[delete(&group.unique_global_id, &random_hex_32())],
        30,
    )
    .await
    .unwrap_err()
    .to_string();
    let unknown_error = group_db::apply_group_sync_batch(
        &db,
        &user,
        &random_hex_32(),
        &[],
        std::slice::from_ref(&unknown),
        &[delete(&unknown.group_global_id, &random_hex_32())],
        30,
    )
    .await
    .unwrap_err()
    .to_string();
    assert_eq!(foreign_error, unknown_error);
    assert_eq!(
        condition(&db, &other, &group.unique_global_id)
            .await
            .expected_state,
        "active"
    );
}

#[tokio::test]
async fn concurrent_conflicting_lifecycle_batches_have_only_one_winner() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let expected = condition(&db, &user, &group.unique_global_id).await;
    let first_db = db.clone();
    let second_db = db.clone();
    let first_user = user.clone();
    let second_user = user.clone();
    let first_global = group.unique_global_id.clone();
    let second_global = first_global.clone();
    let second_expected = expected.clone();
    let first = tokio::spawn(async move {
        group_db::apply_group_sync_batch(
            &first_db,
            &first_user,
            &random_hex_32(),
            &[],
            &[expected],
            &[delete(&first_global, &random_hex_32())],
            30,
        )
        .await
    });
    let second = tokio::spawn(async move {
        group_db::apply_group_sync_batch(
            &second_db,
            &second_user,
            &random_hex_32(),
            &[],
            &[second_expected],
            &[delete(&second_global, &random_hex_32())],
            30,
        )
        .await
    });
    let first = first.await.unwrap();
    let second = second.await.unwrap();
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(
        group_db::get_group_sync_snapshot(&db, &user)
            .await
            .unwrap()
            .deletions
            .len(),
        1
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn keep_restores_recoverable_group_but_restore_rejects_expired_snapshot() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let global = &group.unique_global_id;
    let birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let (id, model_global) = model(&db, &user, "part").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![id], None)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let snapshot = group_db::get_group_sync_snapshot(&db, &user).await.unwrap();
    let deletion = snapshot
        .deletions
        .iter()
        .find(|deletion| &deletion.group_global_id == global)
        .unwrap()
        .operation_id
        .clone();
    let original = condition(&db, &user, global).await;
    let ch = change(&db, &user, &model_global, Some(global)).await;
    assert!(matches!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[ch],
            std::slice::from_ref(&original),
            &[keep(global, &deletion, Some(&birth))],
            30
        )
        .await,
        Ok(AssignmentBatchResult::Applied)
    ));
    assert_eq!(
        assignments(&db, &user).await[0].1.as_deref(),
        Some(global.as_str())
    );
    assert_eq!(
        condition(&db, &user, global).await.expected_generation,
        original.expected_generation
    );
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let second = group_db::get_group_sync_snapshot(&db, &user)
        .await
        .unwrap()
        .deletions
        .into_iter()
        .find(|deletion| deletion.restored_at.is_none())
        .unwrap()
        .operation_id;
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(&db)
    .await
    .unwrap();
    let expired = condition(&db, &user, global).await;
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[],
            std::slice::from_ref(&expired),
            &[restore(global, &second)],
            30
        )
        .await
        .is_err()
    );
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[],
            std::slice::from_ref(&expired),
            &[keep(global, &deletion, Some(&birth))],
            30
        )
        .await
        .is_err(),
        "an old deletion event must not revive a newer deletion"
    );
    assert!(matches!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[],
            &[expired],
            &[keep(global, &second, Some(&birth))],
            30
        )
        .await,
        Ok(AssignmentBatchResult::Applied)
    ));
}

#[tokio::test]
async fn lifecycle_and_assignment_apis_share_idempotency_namespace() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let before = condition(&db, &user, &group.unique_global_id).await;
    let op = random_hex_32();
    assert!(matches!(
        group_db::apply_assignment_batch(&db, &user, &op, &[], &[]).await,
        Ok(AssignmentBatchResult::Applied)
    ));
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &op,
            &[],
            &[before],
            &[delete(&group.unique_global_id, &random_hex_32())],
            30
        )
        .await
        .is_err()
    );
    assert_eq!(
        condition(&db, &user, &group.unique_global_id)
            .await
            .expected_state,
        "active"
    );
}
