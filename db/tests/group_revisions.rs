//! Assignment and group revisions must commit with the state they describe.

use db::{db_context::DbContext, group_db, model::user::User, model_db};

mod common;

async fn model(db: &DbContext, user: &User, name: &str) -> i64 {
    let blob = db::blob_db::add_blob(db, name, "stl", 1, None)
        .await
        .unwrap();
    model_db::add_model(db, user, name, blob, None, None)
        .await
        .unwrap()
}

async fn assert_revision_columns(db: &DbContext) {
    for (table, column) in [
        ("models_group", "group_revision"),
        ("models", "model_assignment_revision"),
    ] {
        let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?)")
            .bind(table)
            .fetch_all(db)
            .await
            .unwrap();
        assert!(
            columns.iter().any(|name| name == column),
            "{table} must have {column}"
        );
    }
}

async fn group_revision(db: &DbContext, id: i64) -> i64 {
    sqlx::query_scalar("SELECT group_revision FROM models_group WHERE group_id = ?")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
}

async fn assignment(db: &DbContext, id: i64) -> (Option<i64>, i64) {
    sqlx::query_as(
        "SELECT model_group_id, model_assignment_revision FROM models WHERE model_id = ?",
    )
    .bind(id)
    .fetch_one(db)
    .await
    .unwrap()
}

#[tokio::test]
async fn moves_and_ungrouping_advance_only_affected_revisions() {
    let (_dir, db) = common::test_db().await;
    assert_revision_columns(&db).await;
    let user = User::default();
    let first_group = group_db::add_empty_group(&db, &user, "A", None)
        .await
        .unwrap();
    let second_group = group_db::add_empty_group(&db, &user, "B", None)
        .await
        .unwrap();
    let other = group_db::add_empty_group(&db, &user, "other", None)
        .await
        .unwrap();
    let moving = model(&db, &user, "moving").await;
    let untouched = model(&db, &user, "untouched").await;
    let initial = assignment(&db, moving).await;
    let initial_a = group_revision(&db, first_group.id).await;
    let initial_b = group_revision(&db, second_group.id).await;
    let initial_other = group_revision(&db, other.id).await;
    let initial_untouched = assignment(&db, untouched).await;

    group_db::set_group_id_on_models(&db, &user, Some(first_group.id), vec![moving, moving], None)
        .await
        .unwrap();
    let after_add = assignment(&db, moving).await;
    let after_a = group_revision(&db, first_group.id).await;
    assert_eq!(after_add.0, Some(first_group.id));
    assert!(after_add.1 > initial.1);
    assert!(after_a > initial_a);
    assert_eq!(group_revision(&db, second_group.id).await, initial_b);
    assert_eq!(group_revision(&db, other.id).await, initial_other);
    assert_eq!(assignment(&db, untouched).await, initial_untouched);

    group_db::set_group_id_on_models(&db, &user, Some(first_group.id), vec![moving], None)
        .await
        .unwrap();
    assert_eq!(assignment(&db, moving).await, after_add);
    assert_eq!(group_revision(&db, first_group.id).await, after_a);

    group_db::set_group_id_on_models(&db, &user, Some(second_group.id), vec![moving], None)
        .await
        .unwrap();
    let after_move = assignment(&db, moving).await;
    let moved_a = group_revision(&db, first_group.id).await;
    let moved_b = group_revision(&db, second_group.id).await;
    assert_eq!(after_move.0, Some(second_group.id));
    assert!(after_move.1 > after_add.1);
    assert!(moved_a > after_a);
    assert!(moved_b > initial_b);
    assert_eq!(group_revision(&db, other.id).await, initial_other);
    assert_eq!(assignment(&db, untouched).await, initial_untouched);

    group_db::set_group_id_on_models(&db, &user, None, vec![moving], None)
        .await
        .unwrap();
    let after_detach = assignment(&db, moving).await;
    let detached_b = group_revision(&db, second_group.id).await;
    assert_eq!(after_detach.0, None);
    assert!(after_detach.1 > after_move.1);
    assert!(detached_b > moved_b);
    assert_eq!(group_revision(&db, first_group.id).await, moved_a);
    group_db::set_group_id_on_models(&db, &user, None, vec![moving], None)
        .await
        .unwrap();
    assert_eq!(assignment(&db, moving).await, after_detach);
    assert_eq!(group_revision(&db, second_group.id).await, detached_b);
}

#[tokio::test]
async fn deleting_and_restoring_a_group_tracks_live_and_trashed_members() {
    let (_dir, db) = common::test_db().await;
    assert_revision_columns(&db).await;
    let user = User::default();
    let source = group_db::add_empty_group(&db, &user, "source", None)
        .await
        .unwrap();
    let destination = group_db::add_empty_group(&db, &user, "destination", None)
        .await
        .unwrap();
    let live = model(&db, &user, "live").await;
    let trashed = model(&db, &user, "trashed").await;
    let moved = model(&db, &user, "moved").await;
    let untouched = model(&db, &user, "untouched").await;
    group_db::set_group_id_on_models(
        &db,
        &user,
        Some(source.id),
        vec![live, trashed, moved],
        None,
    )
    .await
    .unwrap();
    model_db::trash_models(&db, &user, &[trashed])
        .await
        .unwrap();
    let before = [
        assignment(&db, live).await,
        assignment(&db, trashed).await,
        assignment(&db, moved).await,
    ];
    let before_group = group_revision(&db, source.id).await;
    let untouched_before = assignment(&db, untouched).await;
    let destination_before = group_revision(&db, destination.id).await;

    group_db::delete_group(&db, &user, source.id).await.unwrap();
    let deleted = [
        assignment(&db, live).await,
        assignment(&db, trashed).await,
        assignment(&db, moved).await,
    ];
    let deleted_group = group_revision(&db, source.id).await;
    for (old, new) in before.iter().zip(deleted.iter()) {
        assert_eq!(new.0, None);
        assert!(new.1 > old.1);
    }
    assert!(deleted_group > before_group);
    assert_eq!(
        group_revision(&db, destination.id).await,
        destination_before
    );
    assert_eq!(assignment(&db, untouched).await, untouched_before);
    group_db::delete_group(&db, &user, source.id).await.unwrap();
    assert_eq!(group_revision(&db, source.id).await, deleted_group);
    assert_eq!(assignment(&db, live).await, deleted[0]);

    group_db::set_group_id_on_models(&db, &user, Some(destination.id), vec![moved], None)
        .await
        .unwrap();
    let moved_before_restore = assignment(&db, moved).await;
    let destination_before_restore = group_revision(&db, destination.id).await;
    let restore = common::group_restore_request(&db, &user, source.id).await;
    group_db::restore_groups(&db, &user, std::slice::from_ref(&restore), 30)
        .await
        .unwrap();
    let restored_group = group_revision(&db, source.id).await;
    assert!(restored_group > deleted_group);
    for (id, old) in [(live, deleted[0]), (trashed, deleted[1])] {
        let restored = assignment(&db, id).await;
        assert_eq!(restored.0, Some(source.id));
        assert!(restored.1 > old.1);
    }
    assert_eq!(assignment(&db, moved).await, moved_before_restore);
    assert_eq!(
        group_revision(&db, destination.id).await,
        destination_before_restore
    );
    assert_eq!(assignment(&db, untouched).await, untouched_before);
    let live_restored = assignment(&db, live).await;
    group_db::restore_groups(&db, &user, &[restore], 30)
        .await
        .unwrap();
    assert_eq!(group_revision(&db, source.id).await, restored_group);
    assert_eq!(assignment(&db, live).await, live_restored);
}

#[tokio::test]
async fn failing_group_bookkeeping_rolls_back_the_model_move() {
    let (_dir, db) = common::test_db().await;
    assert_revision_columns(&db).await;
    let user = User::default();
    let source = group_db::add_empty_group(&db, &user, "source", None)
        .await
        .unwrap();
    let destination = group_db::add_empty_group(&db, &user, "destination", None)
        .await
        .unwrap();
    let moving = model(&db, &user, "moving").await;
    group_db::set_group_id_on_models(&db, &user, Some(source.id), vec![moving], None)
        .await
        .unwrap();
    let model_before = assignment(&db, moving).await;
    let source_before = group_revision(&db, source.id).await;
    let destination_before = group_revision(&db, destination.id).await;
    sqlx::query("CREATE TRIGGER reject_group_revision BEFORE UPDATE OF group_revision ON models_group BEGIN SELECT RAISE(ABORT, 'injected bookkeeping failure'); END")
        .execute(&db).await.unwrap();

    assert!(
        group_db::set_group_id_on_models(&db, &user, Some(destination.id), vec![moving], None)
            .await
            .is_err()
    );
    assert_eq!(assignment(&db, moving).await, model_before);
    assert_eq!(group_revision(&db, source.id).await, source_before);
    assert_eq!(
        group_revision(&db, destination.id).await,
        destination_before
    );
}

#[tokio::test]
async fn failing_timestamp_bookkeeping_rolls_back_the_model_move() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let source = group_db::add_empty_group(&db, &user, "source", None)
        .await
        .unwrap();
    let destination = group_db::add_empty_group(&db, &user, "destination", None)
        .await
        .unwrap();
    let moving = model(&db, &user, "moving").await;
    group_db::set_group_id_on_models(&db, &user, Some(source.id), vec![moving], None)
        .await
        .unwrap();
    let model_before = assignment(&db, moving).await;
    let source_before = group_revision(&db, source.id).await;
    let destination_before = group_revision(&db, destination.id).await;
    sqlx::query("CREATE TRIGGER reject_group_timestamp BEFORE UPDATE OF group_last_modified ON models_group BEGIN SELECT RAISE(ABORT, 'injected timestamp failure'); END")
        .execute(&db).await.unwrap();

    assert!(
        group_db::set_group_id_on_models(&db, &user, Some(destination.id), vec![moving], None)
            .await
            .is_err()
    );
    assert_eq!(assignment(&db, moving).await, model_before);
    assert_eq!(group_revision(&db, source.id).await, source_before);
    assert_eq!(
        group_revision(&db, destination.id).await,
        destination_before
    );
}

#[tokio::test]
async fn group_metadata_revisions_track_changes_even_with_identical_timestamps() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "original", None)
        .await
        .unwrap();
    let timestamp = "2026-10-24T12:00:00Z";
    let initial = group_revision(&db, group.id).await;
    group_db::edit_group(&db, &user, group.id, "first", Some(timestamp), None)
        .await
        .unwrap();
    let first = group_revision(&db, group.id).await;
    assert!(first > initial);
    group_db::edit_group(&db, &user, group.id, "second", Some(timestamp), None)
        .await
        .unwrap();
    let second = group_revision(&db, group.id).await;
    assert!(second > first);
    group_db::edit_group(&db, &user, group.id, "second", Some(timestamp), None)
        .await
        .unwrap();
    assert_eq!(group_revision(&db, group.id).await, second);
}

#[tokio::test]
async fn resource_assignment_and_deletion_cascade_advance_group_revision() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    let item = model(&db, &user, "resource-model").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![item], None)
        .await
        .unwrap();
    let resource = db::resource_db::add_resource(&db, &user, "resource", None)
        .await
        .unwrap();
    let before = group_revision(&db, group.id).await;
    db::resource_db::set_resource_on_group(&db, &user, Some(resource.id), group.id, None)
        .await
        .unwrap();
    let assigned = group_revision(&db, group.id).await;
    assert!(assigned > before);
    db::resource_db::set_resource_on_group(&db, &user, Some(resource.id), group.id, None)
        .await
        .unwrap();
    assert_eq!(group_revision(&db, group.id).await, assigned);
    db::resource_db::delete_resource(&db, &user, resource.id)
        .await
        .unwrap();
    assert!(group_revision(&db, group.id).await > assigned);
    let resource_id: Option<i64> =
        sqlx::query_scalar("SELECT group_resource_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(resource_id, None);
}

#[tokio::test]
async fn trashing_and_restoring_a_grouped_model_tracks_visibility_not_assignment() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    let item = model(&db, &user, "trash-model").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![item], None)
        .await
        .unwrap();
    let original_assignment = assignment(&db, item).await;
    let before = group_revision(&db, group.id).await;
    model_db::trash_models(&db, &user, &[item]).await.unwrap();
    let trashed = group_revision(&db, group.id).await;
    assert!(trashed > before);
    assert_eq!(assignment(&db, item).await, original_assignment);
    model_db::trash_models(&db, &user, &[item]).await.unwrap();
    assert_eq!(group_revision(&db, group.id).await, trashed);
    model_db::restore_models(&db, &user, &[item]).await.unwrap();
    let restored = group_revision(&db, group.id).await;
    assert!(restored > trashed);
    assert_eq!(assignment(&db, item).await, original_assignment);
    model_db::restore_models(&db, &user, &[item]).await.unwrap();
    assert_eq!(group_revision(&db, group.id).await, restored);
}

#[tokio::test]
async fn deletion_record_retains_its_revision_after_group_purge() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    group_db::edit_group(&db, &user, group.id, "renamed", None, None)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let revision = group_revision(&db, group.id).await;
    let record: i64 = sqlx::query_scalar("SELECT group_revision FROM group_deletion_records WHERE user_id = ? AND group_global_id = ?")
        .bind(user.id).bind(&group.unique_global_id).fetch_one(&db).await.unwrap();
    assert_eq!(record, revision);
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(group_db::purge_trashed_groups(&db, 30).await.unwrap(), 1);
    let retained: i64 = sqlx::query_scalar("SELECT group_revision FROM group_deletion_records WHERE user_id = ? AND group_global_id = ?")
        .bind(user.id).bind(&group.unique_global_id).fetch_one(&db).await.unwrap();
    assert_eq!(retained, record);
}

#[tokio::test]
async fn newly_imported_model_has_a_known_ungrouped_revision() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    let item = model(&db, &user, "imported-model").await;
    let initial = assignment(&db, item).await;
    assert_eq!(initial.0, None);
    assert!(initial.1 >= 0);
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![item], None)
        .await
        .unwrap();
    let grouped = assignment(&db, item).await;
    assert_eq!(grouped.0, Some(group.id));
    assert!(grouped.1 > initial.1);
}
