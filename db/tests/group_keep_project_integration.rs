//! Keep-after-purge must restore the selected project and models atomically.
mod common;

use db::{
    blob_db,
    db_context::DbContext,
    group_db::{
        self, AssignmentBatchResult, AssignmentChange, GroupLifecycleChange, GroupPrecondition,
        KeepResource,
    },
    model::user::User,
    model_db, random_hex_32, resource_db, user_db,
};

async fn prepare(db: &DbContext, user: &User) -> (String, String, String, GroupPrecondition) {
    let group = group_db::add_empty_group(db, user, "original", None)
        .await
        .unwrap();
    let birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(db)
            .await
            .unwrap();
    group_db::delete_group(db, user, group.id).await.unwrap();
    let deletion: String = sqlx::query_scalar("SELECT operation_id FROM group_deletion_records WHERE user_id = ? AND group_global_id = ? AND group_birth_id = ?").bind(user.id).bind(&group.unique_global_id).bind(&birth).fetch_one(db).await.unwrap();
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(db)
    .await
    .unwrap();
    group_db::purge_trashed_groups(db, 30).await.unwrap();
    let snapshot = group_db::get_group_sync_snapshot(db, user).await.unwrap();
    let head = snapshot
        .heads
        .iter()
        .find(|h| h.group_global_id == group.unique_global_id)
        .unwrap();
    let pre = GroupPrecondition {
        group_global_id: group.unique_global_id.clone(),
        expected_generation: head.generation,
        expected_head: head.head,
        expected_state: head.state.clone(),
        expected_revision: None,
        expected_deleted_at: None,
    };
    (group.unique_global_id, birth, deletion, pre)
}

async fn assignment(db: &DbContext, user: &User, global: &str) -> (i64, AssignmentChange) {
    let blob = blob_db::add_blob(db, &random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let id = model_db::add_model(db, user, "model", blob, None, None)
        .await
        .unwrap();
    let (model_global_id, incarnation, revision, lifecycle): (String, String, i64, i64) = sqlx::query_as("SELECT model_unique_global_id, model_incarnation, model_assignment_revision, model_lifecycle_revision FROM models WHERE model_id = ?").bind(id).fetch_one(db).await.unwrap();
    (
        id,
        AssignmentChange {
            model_global_id,
            expected_incarnation: incarnation,
            expected_group_global_id: None,
            expected_revision: revision,
            expected_lifecycle_revision: lifecycle,
            expected_deleted_at: None,
            target_group_global_id: Some(global.into()),
        },
    )
}

async fn resource_identity(db: &DbContext, user: &User, global: &str) -> String {
    resource_db::get_resource_sync_snapshot(db, user)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.global_id == global)
        .unwrap()
        .incarnation
}

fn keep(global: &str, birth: &str, deletion: &str, resource: KeepResource) -> GroupLifecycleChange {
    GroupLifecycleChange::Keep {
        group_global_id: global.into(),
        deletion_operation_id: deletion.into(),
        expected_birth_id: Some(birth.into()),
        name: "kept".into(),
        resource,
    }
}

#[tokio::test]
async fn resource_snapshot_is_owner_scoped_and_incarnation_survives_rename() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let resource = resource_db::add_resource(&db, &user, "project", None)
        .await
        .unwrap();
    let foreign = resource_db::add_resource(&db, &other, "foreign", None)
        .await
        .unwrap();
    let local = resource_db::get_resource_sync_snapshot(&db, &user)
        .await
        .unwrap();
    assert_eq!(local.len(), 1);
    assert_eq!(local[0].global_id, resource.unique_global_id);
    assert_ne!(local[0].incarnation, "");
    assert!(
        !local
            .iter()
            .any(|r| r.global_id == foreign.unique_global_id)
    );
    let identity = local[0].incarnation.clone();
    resource_db::edit_resource(
        &db,
        &user,
        resource.id,
        "renamed",
        resource.flags,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        resource_identity(&db, &user, &resource.unique_global_id).await,
        identity
    );
}

#[tokio::test]
async fn keep_rehydrates_selected_model_and_project_atomically_and_replays() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let resource = resource_db::add_resource(&db, &user, "project", None)
        .await
        .unwrap();
    let resource_incarnation = resource_identity(&db, &user, &resource.unique_global_id).await;
    let (global, birth, deletion, pre) = prepare(&db, &user).await;
    let (model, change) = assignment(&db, &user, &global).await;
    let intent = KeepResource::Existing {
        resource_global_id: resource.unique_global_id.clone(),
        expected_incarnation: resource_incarnation,
    };
    let action = keep(&global, &birth, &deletion, intent);
    let operation = random_hex_32();
    assert!(matches!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &operation,
            std::slice::from_ref(&change),
            std::slice::from_ref(&pre),
            std::slice::from_ref(&action),
            30
        )
        .await
        .unwrap(),
        AssignmentBatchResult::Applied
    ));
    let (id, resource_id): (i64, Option<i64>) = sqlx::query_as("SELECT group_id, group_resource_id FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?").bind(user.id).bind(&global).fetch_one(&db).await.unwrap();
    assert_eq!(resource_id, Some(resource.id));
    let model_group: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
            .bind(model)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(model_group, Some(id));
    let snapshot = group_db::get_group_sync_snapshot(&db, &user).await.unwrap();
    assert_eq!(
        snapshot
            .groups
            .iter()
            .find(|g| g.global_id == global)
            .unwrap()
            .resource_global_id
            .as_deref(),
        Some(resource.unique_global_id.as_str())
    );
    assert!(matches!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &operation,
            std::slice::from_ref(&change),
            std::slice::from_ref(&pre),
            std::slice::from_ref(&action),
            30
        )
        .await
        .unwrap(),
        AssignmentBatchResult::AlreadyApplied
    ));
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &operation,
            &[change],
            &[pre],
            &[keep(&global, &birth, &deletion, KeepResource::None)],
            30
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn keep_explicit_no_project_does_not_invent_an_association() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (global, birth, deletion, pre) = prepare(&db, &user).await;
    let action = keep(&global, &birth, &deletion, KeepResource::None);
    group_db::apply_group_sync_batch(&db, &user, &random_hex_32(), &[], &[pre], &[action], 30)
        .await
        .unwrap();
    let value: Option<i64> = sqlx::query_scalar("SELECT group_resource_id FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?").bind(user.id).bind(&global).fetch_one(&db).await.unwrap();
    assert_eq!(value, None);
}

#[tokio::test]
async fn invalid_foreign_missing_and_stale_projects_reject_without_partial_changes() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let resource = resource_db::add_resource(&db, &user, "project", None)
        .await
        .unwrap();
    let foreign = resource_db::add_resource(&db, &other, "foreign", None)
        .await
        .unwrap();
    let resource_incarnation = resource_identity(&db, &user, &resource.unique_global_id).await;
    let foreign_incarnation = resource_identity(&db, &other, &foreign.unique_global_id).await;
    let (global, birth, deletion, pre) = prepare(&db, &user).await;
    let (id, change) = assignment(&db, &user, &global).await;
    let cases = [
        (random_hex_32(), random_hex_32()),
        (foreign.unique_global_id, foreign_incarnation),
        (resource.unique_global_id.clone(), random_hex_32()),
        (String::new(), resource_incarnation),
    ];
    for (resource_global_id, expected_incarnation) in cases {
        let action = keep(
            &global,
            &birth,
            &deletion,
            KeepResource::Existing {
                resource_global_id,
                expected_incarnation,
            },
        );
        assert!(
            group_db::apply_group_sync_batch(
                &db,
                &user,
                &random_hex_32(),
                std::slice::from_ref(&change),
                std::slice::from_ref(&pre),
                &[action],
                30
            )
            .await
            .is_err()
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?").bind(user.id).bind(&global).fetch_one(&db).await.unwrap();
        assert_eq!(count, 0);
        let assigned: Option<i64> =
            sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(assigned, None);
        let restored: Option<String> = sqlx::query_scalar(
            "SELECT restored_at FROM group_deletion_records WHERE user_id = ? AND operation_id = ?",
        )
        .bind(user.id)
        .bind(&deletion)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(restored, None);
    }
}

#[tokio::test]
async fn duplicate_owned_resource_global_id_fails_snapshot_and_keep() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let first = resource_db::add_resource(&db, &user, "one", None)
        .await
        .unwrap();
    let second = resource_db::add_resource(&db, &user, "two", None)
        .await
        .unwrap();
    let incarnation = resource_identity(&db, &user, &first.unique_global_id).await;
    let (global, birth, deletion, pre) = prepare(&db, &user).await;
    let (id, change) = assignment(&db, &user, &global).await;
    sqlx::query("UPDATE resources SET resource_unique_global_id = ? WHERE resource_id = ?")
        .bind(&first.unique_global_id)
        .bind(second.id)
        .execute(&db)
        .await
        .unwrap();
    assert!(
        resource_db::get_resource_sync_snapshot(&db, &user)
            .await
            .is_err()
    );
    let action = keep(
        &global,
        &birth,
        &deletion,
        KeepResource::Existing {
            resource_global_id: first.unique_global_id,
            expected_incarnation: incarnation,
        },
    );
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[change],
            &[pre],
            &[action],
            30
        )
        .await
        .is_err()
    );
    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(assigned, None);
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn failed_project_update_rolls_back_rehydration_assignment_and_journal() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let resource = resource_db::add_resource(&db, &user, "project", None)
        .await
        .unwrap();
    let incarnation = resource_identity(&db, &user, &resource.unique_global_id).await;
    let (global, birth, deletion, pre) = prepare(&db, &user).await;
    let (id, change) = assignment(&db, &user, &global).await;
    let action = keep(
        &global,
        &birth,
        &deletion,
        KeepResource::Existing {
            resource_global_id: resource.unique_global_id,
            expected_incarnation: incarnation,
        },
    );
    let operation = random_hex_32();
    sqlx::query("CREATE TRIGGER reject_project BEFORE UPDATE OF group_resource_id ON models_group BEGIN SELECT RAISE(ABORT, 'project failure'); END").execute(&db).await.unwrap();
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &operation,
            std::slice::from_ref(&change),
            std::slice::from_ref(&pre),
            std::slice::from_ref(&action),
            30
        )
        .await
        .is_err()
    );
    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(assigned, None);
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(count, 0);
    let restored: Option<String> = sqlx::query_scalar(
        "SELECT restored_at FROM group_deletion_records WHERE user_id = ? AND operation_id = ?",
    )
    .bind(user.id)
    .bind(&deletion)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(restored, None);
    let journal: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM assignment_batch_journal WHERE user_id = ? AND operation_id = ?",
    )
    .bind(user.id)
    .bind(&operation)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(journal, 0);
    sqlx::query("DROP TRIGGER reject_project")
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(
        group_db::apply_group_sync_batch(&db, &user, &operation, &[change], &[pre], &[action], 30)
            .await
            .unwrap(),
        AssignmentBatchResult::Applied
    ));
}

#[tokio::test]
async fn recreated_project_with_same_global_id_does_not_match_stale_incarnation() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let old = resource_db::add_resource(&db, &user, "old", None)
        .await
        .unwrap();
    let old_incarnation = resource_identity(&db, &user, &old.unique_global_id).await;
    resource_db::delete_resource(&db, &user, old.id)
        .await
        .unwrap();
    let replacement = resource_db::add_resource(&db, &user, "replacement", None)
        .await
        .unwrap();
    sqlx::query("UPDATE resources SET resource_unique_global_id = ? WHERE resource_id = ?")
        .bind(&old.unique_global_id)
        .bind(replacement.id)
        .execute(&db)
        .await
        .unwrap();
    assert_ne!(
        resource_identity(&db, &user, &old.unique_global_id).await,
        old_incarnation
    );
    let (global, birth, deletion, pre) = prepare(&db, &user).await;
    let (id, change) = assignment(&db, &user, &global).await;
    let action = keep(
        &global,
        &birth,
        &deletion,
        KeepResource::Existing {
            resource_global_id: old.unique_global_id,
            expected_incarnation: old_incarnation,
        },
    );
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[change],
            &[pre],
            &[action],
            30
        )
        .await
        .is_err()
    );
    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(assigned, None);
}

#[tokio::test]
async fn keep_existing_trashed_group_can_explicitly_clear_its_old_project() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let resource = resource_db::add_resource(&db, &user, "project", None)
        .await
        .unwrap();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    let (id, _) = assignment(&db, &user, &group.unique_global_id).await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![id], None)
        .await
        .unwrap();
    resource_db::set_resource_on_group(&db, &user, Some(resource.id), group.id, None)
        .await
        .unwrap();
    let birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let deletion: String = sqlx::query_scalar(
        "SELECT operation_id FROM group_deletion_records WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&group.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    let snapshot = group_db::get_group_sync_snapshot(&db, &user).await.unwrap();
    let head = snapshot
        .heads
        .iter()
        .find(|h| h.group_global_id == group.unique_global_id)
        .unwrap();
    let current = snapshot
        .groups
        .iter()
        .find(|g| g.global_id == group.unique_global_id)
        .unwrap();
    let pre = GroupPrecondition {
        group_global_id: group.unique_global_id.clone(),
        expected_generation: head.generation,
        expected_head: head.head,
        expected_state: head.state.clone(),
        expected_revision: Some(current.revision),
        expected_deleted_at: current.deleted_at.clone(),
    };
    let action = keep(
        &group.unique_global_id,
        &birth,
        &deletion,
        KeepResource::None,
    );
    group_db::apply_group_sync_batch(&db, &user, &random_hex_32(), &[], &[pre], &[action], 30)
        .await
        .unwrap();
    let project: Option<i64> =
        sqlx::query_scalar("SELECT group_resource_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(project, None);
    assert!(
        resource_db::get_resources(&db, &user)
            .await
            .unwrap()
            .iter()
            .any(|r| r.id == resource.id)
    );
}
