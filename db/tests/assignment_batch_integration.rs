//! Assignment batches must be atomic, owner-scoped and safe to retry.
mod common;

use db::{
    blob_db,
    db_context::DbContext,
    group_db::{self, AssignmentBatchResult, AssignmentChange, GroupPrecondition},
    model::user::User,
    model_db, random_hex_32, user_db,
};

async fn model(db: &DbContext, user: &User, name: &str) -> (i64, String) {
    let blob = blob_db::add_blob(db, name, "stl", 1, None).await.unwrap();
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

async fn change(db: &DbContext, user: &User, id: &str, target: Option<&str>) -> AssignmentChange {
    let snapshot = group_db::get_group_sync_snapshot(db, user).await.unwrap();
    let item = snapshot
        .assignments
        .iter()
        .find(|m| m.model_global_id == id)
        .unwrap();
    AssignmentChange {
        model_global_id: id.to_owned(),
        expected_incarnation: item.incarnation.clone(),
        expected_group_global_id: item.group_global_id.clone(),
        expected_revision: item.revision,
        expected_lifecycle_revision: item.lifecycle_revision,
        expected_deleted_at: item.deleted_at.clone(),
        target_group_global_id: target.map(str::to_owned),
    }
}

async fn precondition(db: &DbContext, user: &User, id: &str) -> GroupPrecondition {
    let snapshot = group_db::get_group_sync_snapshot(db, user).await.unwrap();
    let group = snapshot.groups.iter().find(|g| g.global_id == id).unwrap();
    let head = snapshot
        .heads
        .iter()
        .find(|h| h.group_global_id == id)
        .unwrap();
    GroupPrecondition {
        group_global_id: id.to_owned(),
        expected_generation: head.generation,
        expected_head: head.head,
        expected_state: head.state.clone(),
        expected_revision: Some(group.revision),
        expected_deleted_at: group.deleted_at.clone(),
    }
}

async fn snapshot(db: &DbContext, user: &User) -> String {
    let snapshot = group_db::get_group_sync_snapshot(db, user).await.unwrap();
    let groups: Vec<_> = snapshot
        .groups
        .into_iter()
        .map(|g| (g.global_id, g.name, g.revision, g.deleted_at))
        .collect();
    let assignments: Vec<_> = snapshot
        .assignments
        .into_iter()
        .map(|m| {
            (
                m.model_global_id,
                m.incarnation,
                m.group_global_id,
                m.revision,
                m.deleted_at,
            )
        })
        .collect();
    let deletions: Vec<_> = snapshot
        .deletions
        .into_iter()
        .map(|d| {
            (
                d.group_global_id,
                d.operation_id,
                d.deleted_at,
                d.restored_at,
                d.revision,
            )
        })
        .collect();
    let heads: Vec<_> = snapshot
        .heads
        .into_iter()
        .map(|h| (h.group_global_id, h.generation, h.head, h.state))
        .collect();
    format!("{groups:?}{assignments:?}{deletions:?}{heads:?}")
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn batch_is_atomic_and_exact_retry_is_idempotent_and_order_independent() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let source = group_db::add_empty_group(&db, &user, "source", None)
        .await
        .unwrap();
    let target = group_db::add_empty_group(&db, &user, "target", None)
        .await
        .unwrap();
    let (first, first_global) = model(&db, &user, "first").await;
    let (second, second_global) = model(&db, &user, "second").await;
    group_db::set_group_id_on_models(&db, &user, Some(source.id), vec![first, second], None)
        .await
        .unwrap();
    let one = change(&db, &user, &first_global, Some(&target.unique_global_id)).await;
    let two = change(&db, &user, &second_global, Some(&target.unique_global_id)).await;
    let source_before = precondition(&db, &user, &source.unique_global_id).await;
    let target_before = precondition(&db, &user, &target.unique_global_id).await;
    let op = random_hex_32();
    let result = group_db::apply_assignment_batch(
        &db,
        &user,
        &op,
        &[one.clone(), two.clone()],
        &[source_before.clone(), target_before.clone()],
    )
    .await
    .unwrap();
    assert!(matches!(result, AssignmentBatchResult::Applied));
    let applied = snapshot(&db, &user).await;
    let after = group_db::get_group_sync_snapshot(&db, &user).await.unwrap();
    for global in [&first_global, &second_global] {
        let item = after
            .assignments
            .iter()
            .find(|m| &m.model_global_id == global)
            .unwrap();
        assert_eq!(
            item.group_global_id.as_deref(),
            Some(target.unique_global_id.as_str())
        );
    }
    assert!(
        after
            .groups
            .iter()
            .find(|g| g.global_id == source.unique_global_id)
            .unwrap()
            .revision
            > source_before.expected_revision.unwrap()
    );
    assert!(
        after
            .groups
            .iter()
            .find(|g| g.global_id == target.unique_global_id)
            .unwrap()
            .revision
            > target_before.expected_revision.unwrap()
    );
    let retry = group_db::apply_assignment_batch(
        &db,
        &user,
        &op,
        &[two.clone(), one.clone()],
        &[target_before.clone(), source_before.clone()],
    )
    .await
    .unwrap();
    assert!(matches!(retry, AssignmentBatchResult::AlreadyApplied));
    assert_eq!(snapshot(&db, &user).await, applied);
    let mut wrong_incarnation = one.clone();
    wrong_incarnation.expected_incarnation = random_hex_32();
    assert!(
        group_db::apply_assignment_batch(
            &db,
            &user,
            &op,
            &[wrong_incarnation, two.clone()],
            &[source_before.clone(), target_before.clone()]
        )
        .await
        .is_err()
    );
    let mut altered = one.clone();
    altered.target_group_global_id = None;
    assert!(
        group_db::apply_assignment_batch(
            &db,
            &user,
            &op,
            &[altered, two.clone()],
            &[source_before.clone(), target_before.clone()]
        )
        .await
        .is_err()
    );
    assert_eq!(snapshot(&db, &user).await, applied);

    // A lost-response retry never undoes an intervening later move.
    let later = change(&db, &user, &first_global, None).await;
    let target_current = precondition(&db, &user, &target.unique_global_id).await;
    group_db::apply_assignment_batch(&db, &user, &random_hex_32(), &[later], &[target_current])
        .await
        .unwrap();
    let later_state = snapshot(&db, &user).await;
    assert!(matches!(
        group_db::apply_assignment_batch(
            &db,
            &user,
            &op,
            &[one, two],
            &[source_before, target_before]
        )
        .await
        .unwrap(),
        AssignmentBatchResult::AlreadyApplied
    ));
    assert_eq!(snapshot(&db, &user).await, later_state);
}

#[tokio::test]
async fn stale_invalid_and_foreign_preconditions_reject_the_entire_batch() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let foreign_id = user_db::add_user(&db, "foreign", "foreign@example.test", &random_hex_32())
        .await
        .unwrap();
    let foreign = user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let target = group_db::add_empty_group(&db, &owner, "target", None)
        .await
        .unwrap();
    let foreign_group = group_db::add_empty_group(&db, &foreign, "foreign", None)
        .await
        .unwrap();
    let (_, first) = model(&db, &owner, "owned-first").await;
    let (_, second) = model(&db, &owner, "owned-second").await;
    let (_, other) = model(&db, &foreign, "foreign-model").await;
    let first_change = change(&db, &owner, &first, Some(&target.unique_global_id)).await;
    let second_change = change(&db, &owner, &second, Some(&target.unique_global_id)).await;
    let target_pre = precondition(&db, &owner, &target.unique_global_id).await;
    let before = snapshot(&db, &owner).await;
    let foreign_before = snapshot(&db, &foreign).await;
    let mut stale = second_change.clone();
    stale.expected_revision += 1;
    let mut bad_group = target_pre.clone();
    bad_group.expected_revision = bad_group.expected_revision.map(|revision| revision + 1);
    let mut bad_state = target_pre.clone();
    bad_state.expected_state = "trashed".into();
    let mut foreign_model = first_change.clone();
    foreign_model.model_global_id = other;
    let mut foreign_target = first_change.clone();
    foreign_target.target_group_global_id = Some(foreign_group.unique_global_id.clone());
    let foreign_pre = precondition(&db, &foreign, &foreign_group.unique_global_id).await;
    for (changes, groups) in [
        (vec![first_change.clone(), stale], vec![target_pre.clone()]),
        (
            vec![first_change.clone(), second_change.clone()],
            vec![bad_group],
        ),
        (vec![first_change.clone()], vec![bad_state]),
        (
            vec![first_change.clone(), foreign_model],
            vec![target_pre.clone()],
        ),
        (vec![foreign_target], vec![foreign_pre]),
        (
            vec![first_change.clone(), first_change.clone()],
            vec![target_pre.clone()],
        ),
        (vec![first_change.clone()], vec![]),
    ] {
        assert!(
            group_db::apply_assignment_batch(&db, &owner, &random_hex_32(), &changes, &groups)
                .await
                .is_err()
        );
        assert_eq!(snapshot(&db, &owner).await, before);
        assert_eq!(snapshot(&db, &foreign).await, foreign_before);
    }
    group_db::delete_group(&db, &owner, target.id)
        .await
        .unwrap();
    let deleted_state = snapshot(&db, &owner).await;
    assert!(
        group_db::apply_assignment_batch(
            &db,
            &owner,
            &random_hex_32(),
            &[first_change],
            &[target_pre]
        )
        .await
        .is_err()
    );
    assert_eq!(snapshot(&db, &owner).await, deleted_state);
}

#[tokio::test]
async fn no_op_does_not_change_revisions_but_operation_is_replayable() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (_, global) = model(&db, &user, "no-op").await;
    let change = change(&db, &user, &global, None).await;
    let before = snapshot(&db, &user).await;
    let op = random_hex_32();
    assert!(matches!(
        group_db::apply_assignment_batch(&db, &user, &op, std::slice::from_ref(&change), &[])
            .await
            .unwrap(),
        AssignmentBatchResult::Applied
    ));
    assert_eq!(snapshot(&db, &user).await, before);
    assert!(matches!(
        group_db::apply_assignment_batch(&db, &user, &op, &[change], &[])
            .await
            .unwrap(),
        AssignmentBatchResult::AlreadyApplied
    ));
    assert_eq!(snapshot(&db, &user).await, before);
}

#[tokio::test]
async fn injected_failure_rolls_back_all_changes_and_allows_retry() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let target = group_db::add_empty_group(&db, &user, "target", None)
        .await
        .unwrap();
    let (_, first) = model(&db, &user, "rollback-first").await;
    let (second_id, second) = model(&db, &user, "rollback-second").await;
    let one = change(&db, &user, &first, Some(&target.unique_global_id)).await;
    let two = change(&db, &user, &second, Some(&target.unique_global_id)).await;
    let group = precondition(&db, &user, &target.unique_global_id).await;
    let before = snapshot(&db, &user).await;
    let op = random_hex_32();
    sqlx::query("CREATE TRIGGER reject_second_assignment BEFORE UPDATE OF model_group_id ON models WHEN OLD.model_name = 'rollback-second' BEGIN SELECT RAISE(ABORT, 'injected failure'); END").execute(&db).await.unwrap();
    assert!(
        group_db::apply_assignment_batch(
            &db,
            &user,
            &op,
            &[one.clone(), two.clone()],
            std::slice::from_ref(&group)
        )
        .await
        .is_err()
    );
    assert_eq!(snapshot(&db, &user).await, before);
    sqlx::query("DROP TRIGGER reject_second_assignment")
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(
        group_db::apply_assignment_batch(&db, &user, &op, &[one, two], &[group])
            .await
            .unwrap(),
        AssignmentBatchResult::Applied
    ));
    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
            .bind(second_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(assigned, Some(target.id));
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_competing_assignments_cannot_both_succeed() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let a = group_db::add_empty_group(&db, &user, "A", None)
        .await
        .unwrap();
    let b = group_db::add_empty_group(&db, &user, "B", None)
        .await
        .unwrap();
    let (_, global) = model(&db, &user, "race").await;
    let to_a = change(&db, &user, &global, Some(&a.unique_global_id)).await;
    let to_b = change(&db, &user, &global, Some(&b.unique_global_id)).await;
    let pre_a = precondition(&db, &user, &a.unique_global_id).await;
    let pre_b = precondition(&db, &user, &b.unique_global_id).await;
    let op_a = random_hex_32();
    let op_b = random_hex_32();
    let changes_a = [to_a];
    let changes_b = [to_b];
    let groups_a = [pre_a];
    let groups_b = [pre_b];
    let (left, right) = tokio::join!(
        group_db::apply_assignment_batch(&db, &user, &op_a, &changes_a, &groups_a),
        group_db::apply_assignment_batch(&db, &user, &op_b, &changes_b, &groups_b),
    );
    assert_ne!(
        left.is_ok(),
        right.is_ok(),
        "exactly one competing operation may commit"
    );
    let after = group_db::get_group_sync_snapshot(&db, &user).await.unwrap();
    let item = after
        .assignments
        .iter()
        .find(|m| m.model_global_id == global)
        .unwrap();
    let winner = if left.is_ok() {
        &a.unique_global_id
    } else {
        &b.unique_global_id
    };
    assert_eq!(item.group_global_id.as_deref(), Some(winner.as_str()));
}

#[tokio::test]
async fn stale_group_precondition_rejects_purged_then_recreated_identity() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "original", None)
        .await
        .unwrap();
    let (_, global) = model(&db, &user, "aba-model").await;
    let stale_change = change(&db, &user, &global, Some(&group.unique_global_id)).await;
    let mut stale_group = precondition(&db, &user, &group.unique_global_id).await;

    group_db::delete_group(&db, &user, group.id).await.unwrap();
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(group_db::purge_trashed_groups(&db, 30).await.unwrap(), 1);
    sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id) VALUES (?, ?, ?, ?, ?)")
        .bind("recreated").bind("2026-01-01T00:00:00Z").bind(user.id).bind("2026-01-01T00:00:00Z").bind(&group.unique_global_id)
        .execute(&db).await.unwrap();
    let recreated = precondition(&db, &user, &group.unique_global_id).await;
    assert_eq!(recreated.expected_revision, stale_group.expected_revision);
    assert_eq!(
        recreated.expected_deleted_at,
        stale_group.expected_deleted_at
    );
    assert_ne!(
        recreated.expected_generation,
        stale_group.expected_generation
    );
    // Isolate generation from the independently checked head.
    stale_group.expected_head = recreated.expected_head;
    let before = snapshot(&db, &user).await;

    assert!(
        group_db::apply_assignment_batch(
            &db,
            &user,
            &random_hex_32(),
            &[stale_change],
            &[stale_group]
        )
        .await
        .is_err(),
        "a recreated group must not match stale generation"
    );
    assert_eq!(snapshot(&db, &user).await, before);
}

#[tokio::test]
async fn stale_model_precondition_rejects_purged_then_recreated_identity() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "target", None)
        .await
        .unwrap();
    let (old_id, global) = model(&db, &user, "original-model").await;
    let stale_change = change(&db, &user, &global, Some(&group.unique_global_id)).await;
    let target = precondition(&db, &user, &group.unique_global_id).await;

    sqlx::query("DELETE FROM models WHERE model_id = ? AND model_user_id = ?")
        .bind(old_id)
        .bind(user.id)
        .execute(&db)
        .await
        .unwrap();
    let (new_id, _) = model(&db, &user, "replacement-model").await;
    sqlx::query(
        "UPDATE models SET model_unique_global_id = ? WHERE model_id = ? AND model_user_id = ?",
    )
    .bind(&global)
    .bind(new_id)
    .bind(user.id)
    .execute(&db)
    .await
    .unwrap();
    let recreated = change(&db, &user, &global, Some(&group.unique_global_id)).await;
    assert_eq!(recreated.expected_revision, stale_change.expected_revision);
    assert_eq!(
        recreated.expected_group_global_id,
        stale_change.expected_group_global_id
    );
    assert_eq!(
        recreated.expected_deleted_at,
        stale_change.expected_deleted_at
    );
    assert_ne!(
        recreated.expected_incarnation,
        stale_change.expected_incarnation
    );
    let before = snapshot(&db, &user).await;

    assert!(
        group_db::apply_assignment_batch(&db, &user, &random_hex_32(), &[stale_change], &[target])
            .await
            .is_err(),
        "a recreated model must not match stale incarnation"
    );
    assert_eq!(snapshot(&db, &user).await, before);
}

#[tokio::test]
async fn model_incarnation_is_nonempty_unique_and_stable_across_global_id_changes() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (id, first_global) = model(&db, &user, "incarnation-one").await;
    let (_, second_global) = model(&db, &user, "incarnation-two").await;
    let first = change(&db, &user, &first_global, None).await;
    let second = change(&db, &user, &second_global, None).await;
    assert_ne!(first.expected_incarnation, "");
    assert_ne!(first.expected_incarnation, second.expected_incarnation);
    let new_global = random_hex_32();
    sqlx::query(
        "UPDATE models SET model_unique_global_id = ? WHERE model_id = ? AND model_user_id = ?",
    )
    .bind(&new_global)
    .bind(id)
    .bind(user.id)
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(
        change(&db, &user, &new_global, None)
            .await
            .expected_incarnation,
        first.expected_incarnation
    );
}

#[tokio::test]
async fn stale_group_head_rejects_even_a_no_op_assignment() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    let (id, global) = model(&db, &user, "stale-head").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![id], None)
        .await
        .unwrap();
    let unchanged = change(&db, &user, &global, Some(&group.unique_global_id)).await;
    let mut stale = precondition(&db, &user, &group.unique_global_id).await;
    group_db::edit_group(&db, &user, group.id, "renamed", None, None)
        .await
        .unwrap();
    let current = precondition(&db, &user, &group.unique_global_id).await;
    assert_ne!(stale.expected_head, current.expected_head);
    stale.expected_revision = current.expected_revision;
    stale.expected_deleted_at = current.expected_deleted_at;
    let before = snapshot(&db, &user).await;
    assert!(
        group_db::apply_assignment_batch(&db, &user, &random_hex_32(), &[unchanged], &[stale])
            .await
            .is_err()
    );
    assert_eq!(snapshot(&db, &user).await, before);
}
