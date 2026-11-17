//! A purged group may be kept only with its exact retained birth identity.
mod common;

use db::{
    blob_db,
    db_context::DbContext,
    group_db::{
        self, AssignmentBatchResult, AssignmentChange, GroupLifecycleChange, GroupPrecondition,
    },
    model::user::User,
    model_db, random_hex_32, user_db,
};

async fn prepare(db: &DbContext, user: &User) -> (String, String, String) {
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
    let (operation, recorded_birth): (String, Option<String>) = sqlx::query_as("SELECT operation_id, group_birth_id FROM group_deletion_records WHERE user_id = ? AND group_global_id = ?")
        .bind(user.id).bind(&group.unique_global_id).fetch_one(db).await.unwrap();
    assert_eq!(recorded_birth.as_deref(), Some(birth.as_str()));
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(db)
    .await
    .unwrap();
    assert_eq!(group_db::purge_trashed_groups(db, 30).await.unwrap(), 1);
    (group.unique_global_id, birth, operation)
}

async fn precondition(db: &DbContext, user: &User, global: &str) -> GroupPrecondition {
    let snapshot = group_db::get_group_sync_snapshot(db, user).await.unwrap();
    let head = snapshot
        .heads
        .iter()
        .find(|h| h.group_global_id == global)
        .unwrap();
    GroupPrecondition {
        group_global_id: global.into(),
        expected_generation: head.generation,
        expected_head: head.head,
        expected_state: head.state.clone(),
        expected_revision: None,
        expected_deleted_at: None,
    }
}

async fn new_model(
    db: &DbContext,
    user: &User,
    name: &str,
    target: &str,
) -> (i64, AssignmentChange) {
    let blob = blob_db::add_blob(db, name, "stl", 1, None).await.unwrap();
    let id = model_db::add_model(db, user, name, blob, None, None)
        .await
        .unwrap();
    let (global, incarnation, revision, lifecycle): (String, String, i64, i64) = sqlx::query_as("SELECT model_unique_global_id, model_incarnation, model_assignment_revision, model_lifecycle_revision FROM models WHERE model_id = ?").bind(id).fetch_one(db).await.unwrap();
    (
        id,
        AssignmentChange {
            model_global_id: global,
            expected_incarnation: incarnation,
            expected_group_global_id: None,
            expected_revision: revision,
            expected_lifecycle_revision: lifecycle,
            expected_deleted_at: None,
            target_group_global_id: Some(target.into()),
        },
    )
}

fn keep(global: &str, deletion: &str, birth: Option<&str>) -> GroupLifecycleChange {
    GroupLifecycleChange::Keep {
        group_global_id: global.into(),
        deletion_operation_id: deletion.into(),
        expected_birth_id: birth.map(str::to_owned),
        resource: group_db::KeepResource::None,
        name: "kept".into(),
    }
}

#[tokio::test]
async fn exact_birth_keeps_purged_group_with_only_selected_assignment_and_safe_replay() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (global, birth, deletion) = prepare(&db, &user).await;
    let (selected_id, selected) = new_model(&db, &user, "selected", &global).await;
    let (unselected_id, _) = new_model(&db, &user, "unselected", &global).await;
    let pre = precondition(&db, &user, &global).await;
    let action = keep(&global, &deletion, Some(&birth));
    let operation = random_hex_32();
    assert!(matches!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &operation,
            std::slice::from_ref(&selected),
            std::slice::from_ref(&pre),
            std::slice::from_ref(&action),
            30
        )
        .await
        .unwrap(),
        AssignmentBatchResult::Applied
    ));
    let (id, name, actual_birth): (i64, String, Option<String>) = sqlx::query_as("SELECT group_id, group_name, group_birth_id FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ? AND group_deleted_at IS NULL").bind(user.id).bind(&global).fetch_one(&db).await.unwrap();
    assert_eq!(name, "kept");
    assert_eq!(actual_birth.as_deref(), Some(birth.as_str()));
    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
            .bind(selected_id)
            .fetch_one(&db)
            .await
            .unwrap();
    let untouched: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
            .bind(unselected_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(assigned, Some(id));
    assert_eq!(untouched, None);
    let (count, restored): (i64, Option<String>) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM entity_births WHERE user_id = ? AND entity_kind = 'group' AND birth_id = ? AND global_id = ?), (SELECT restored_at FROM group_deletion_records WHERE user_id = ? AND operation_id = ?)")
        .bind(user.id).bind(&birth).bind(&global).bind(user.id).bind(&deletion).fetch_one(&db).await.unwrap();
    assert_eq!(count, 1);
    assert!(restored.is_some());
    let revision: i64 =
        sqlx::query_scalar("SELECT group_revision FROM models_group WHERE group_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(matches!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &operation,
            std::slice::from_ref(&selected),
            std::slice::from_ref(&pre),
            &[action],
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
            &[selected],
            &[pre],
            &[keep(&global, &deletion, None)],
            30
        )
        .await
        .is_err()
    );
    let after: i64 =
        sqlx::query_scalar("SELECT group_revision FROM models_group WHERE group_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(after, revision);
}

#[tokio::test]
async fn mismatched_or_omitted_birth_rejects_without_partial_changes() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let (global, birth, deletion) = prepare(&db, &owner).await;
    let (model, selected) = new_model(&db, &owner, "selected", &global).await;
    let pre = precondition(&db, &owner, &global).await;
    for wrong in [None, Some(random_hex_32())] {
        let action = keep(&global, &deletion, wrong.as_deref());
        assert!(
            group_db::apply_group_sync_batch(
                &db,
                &owner,
                &random_hex_32(),
                std::slice::from_ref(&selected),
                std::slice::from_ref(&pre),
                &[action],
                30
            )
            .await
            .is_err()
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?").bind(owner.id).bind(&global).fetch_one(&db).await.unwrap();
        assert_eq!(count, 0);
        let assigned: Option<i64> =
            sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
                .bind(model)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(assigned, None);
        let restored: Option<String> = sqlx::query_scalar(
            "SELECT restored_at FROM group_deletion_records WHERE user_id = ? AND operation_id = ?",
        )
        .bind(owner.id)
        .bind(&deletion)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(restored, None);
        let retained: String = sqlx::query_scalar("SELECT global_id FROM entity_births WHERE user_id = ? AND entity_kind = 'group' AND birth_id = ?").bind(owner.id).bind(&birth).fetch_one(&db).await.unwrap();
        assert_eq!(retained, global);
    }
}

#[tokio::test]
async fn legacy_unknown_birth_remains_unknown_and_cannot_adopt_a_claimed_birth() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let global = random_hex_32();
    let group = sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id) VALUES ('legacy', '2020-01-01T00:00:00Z', ?, '2020-01-01T00:00:00Z', ?)")
        .bind(user.id).bind(&global).execute(&db).await.unwrap().last_insert_rowid();
    group_db::delete_group(&db, &user, group).await.unwrap();
    let (deletion, birth): (String, Option<String>) = sqlx::query_as("SELECT operation_id, group_birth_id FROM group_deletion_records WHERE user_id = ? AND group_global_id = ?").bind(user.id).bind(&global).fetch_one(&db).await.unwrap();
    assert_eq!(birth, None);
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group)
    .execute(&db)
    .await
    .unwrap();
    group_db::purge_trashed_groups(&db, 30).await.unwrap();
    let pre = precondition(&db, &user, &global).await;
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[],
            std::slice::from_ref(&pre),
            &[keep(&global, &deletion, Some(&random_hex_32()))],
            30
        )
        .await
        .is_err()
    );
    group_db::apply_group_sync_batch(
        &db,
        &user,
        &random_hex_32(),
        &[],
        &[pre],
        &[keep(&global, &deletion, None)],
        30,
    )
    .await
    .unwrap();
    let result: Option<String> = sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?").bind(user.id).bind(&global).fetch_one(&db).await.unwrap();
    assert_eq!(result, None);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entity_births WHERE user_id = ? AND entity_kind = 'group' AND global_id = ?").bind(user.id).bind(&global).fetch_one(&db).await.unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn rehydration_and_assignment_roll_back_together_on_late_error() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (global, birth, deletion) = prepare(&db, &user).await;
    let (id, selected) = new_model(&db, &user, "failure", &global).await;
    let pre = precondition(&db, &user, &global).await;
    let action = keep(&global, &deletion, Some(&birth));
    let op = random_hex_32();
    sqlx::query("CREATE TRIGGER reject_keep_assignment BEFORE UPDATE OF model_group_id ON models BEGIN SELECT RAISE(ABORT, 'injected assignment failure'); END").execute(&db).await.unwrap();
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &op,
            std::slice::from_ref(&selected),
            std::slice::from_ref(&pre),
            std::slice::from_ref(&action),
            30
        )
        .await
        .is_err()
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&db)
    .await
    .unwrap();
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
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entity_births WHERE user_id = ? AND entity_kind = 'group' AND birth_id = ? AND global_id = ?").bind(user.id).bind(&birth).bind(&global).fetch_one(&db).await.unwrap();
    assert_eq!(count, 1);
    sqlx::query("DROP TRIGGER reject_keep_assignment")
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(
        group_db::apply_group_sync_batch(&db, &user, &op, &[selected], &[pre], &[action], 30)
            .await
            .unwrap(),
        AssignmentBatchResult::Applied
    ));
}

#[tokio::test]
async fn another_owner_cannot_keep_a_foreign_deletion_or_birth() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let foreign_id = user_db::add_user(&db, "foreign", "foreign@example.test", &random_hex_32())
        .await
        .unwrap();
    let foreign = user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let (global, birth, deletion) = prepare(&db, &owner).await;
    let pre = precondition(&db, &owner, &global).await;
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &foreign,
            &random_hex_32(),
            &[],
            &[pre],
            &[keep(&global, &deletion, Some(&birth))],
            30
        )
        .await
        .is_err()
    );
    let restored: Option<String> = sqlx::query_scalar(
        "SELECT restored_at FROM group_deletion_records WHERE user_id = ? AND operation_id = ?",
    )
    .bind(owner.id)
    .bind(&deletion)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(restored, None);
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM models_group WHERE group_unique_global_id = ?")
            .bind(&global)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn retained_birth_with_a_conflicting_registry_mapping_cannot_be_rehydrated() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (global, birth, deletion) = prepare(&db, &user).await;
    let other_global = random_hex_32();
    sqlx::query("UPDATE entity_births SET global_id = ? WHERE user_id = ? AND entity_kind = 'group' AND birth_id = ?")
        .bind(&other_global).bind(user.id).bind(&birth).execute(&db).await.unwrap();
    let pre = precondition(&db, &user, &global).await;
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[],
            &[pre],
            &[keep(&global, &deletion, Some(&birth))],
            30
        )
        .await
        .is_err()
    );
    let mapping: String = sqlx::query_scalar("SELECT global_id FROM entity_births WHERE user_id = ? AND entity_kind = 'group' AND birth_id = ?").bind(user.id).bind(&birth).fetch_one(&db).await.unwrap();
    assert_eq!(mapping, other_global);
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
async fn registry_match_cannot_override_the_deletion_records_birth() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let (global, birth, deletion) = prepare(&db, &user).await;
    let wrong = random_hex_32();
    sqlx::query("INSERT INTO entity_births (user_id, entity_kind, birth_id, global_id) VALUES (?, 'group', ?, ?)")
        .bind(user.id).bind(&wrong).bind(&global).execute(&db).await.unwrap();
    let pre = precondition(&db, &user, &global).await;
    assert!(
        group_db::apply_group_sync_batch(
            &db,
            &user,
            &random_hex_32(),
            &[],
            &[pre],
            &[keep(&global, &deletion, Some(&wrong))],
            30
        )
        .await
        .is_err()
    );
    let recorded: String = sqlx::query_scalar(
        "SELECT group_birth_id FROM group_deletion_records WHERE user_id = ? AND operation_id = ?",
    )
    .bind(user.id)
    .bind(&deletion)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(recorded, birth);
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?",
    )
    .bind(user.id)
    .bind(global)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(count, 0);
}
