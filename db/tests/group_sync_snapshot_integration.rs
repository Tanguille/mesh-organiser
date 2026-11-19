//! A sync snapshot must represent one owner's complete, consistent group state.

use db::{group_db, model::user::User, model_db, user_db};

mod common;

async fn add_model(db: &db::db_context::DbContext, user: &User, name: &str) -> (i64, String) {
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

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn snapshot_contains_complete_owner_state_and_retains_deletion_history() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let foreign_id =
        user_db::add_user(&db, "foreign", "foreign@example.test", &db::random_hex_32())
            .await
            .unwrap();
    let foreign = user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let active = group_db::add_empty_group(&db, &owner, "active", None)
        .await
        .unwrap();
    let deleted = group_db::add_empty_group(&db, &owner, "deleted", None)
        .await
        .unwrap();
    let purged = group_db::add_empty_group(&db, &owner, "purged", None)
        .await
        .unwrap();
    let foreign_group = group_db::add_empty_group(&db, &foreign, "foreign", None)
        .await
        .unwrap();
    let (live_id, live_global) = add_model(&db, &owner, "live").await;
    let (trash_id, trash_global) = add_model(&db, &owner, "trash").await;
    let (ungrouped_id, ungrouped_global) = add_model(&db, &owner, "ungrouped").await;
    let (foreign_model, foreign_global) = add_model(&db, &foreign, "foreign-model").await;
    group_db::set_group_id_on_models(&db, &owner, Some(active.id), vec![live_id, trash_id], None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(
        &db,
        &foreign,
        Some(foreign_group.id),
        vec![foreign_model],
        None,
    )
    .await
    .unwrap();
    model_db::trash_models(&db, &owner, &[trash_id])
        .await
        .unwrap();
    group_db::delete_group(&db, &owner, deleted.id)
        .await
        .unwrap();
    group_db::delete_group(&db, &owner, purged.id)
        .await
        .unwrap();
    group_db::delete_group(&db, &foreign, foreign_group.id)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(purged.id)
    .execute(&db)
    .await
    .unwrap();
    group_db::purge_trashed_groups(&db, 30).await.unwrap();

    let snapshot = group_db::get_group_sync_snapshot(&db, &owner)
        .await
        .unwrap();
    assert_eq!(snapshot.groups.len(), 2);
    let active_row = snapshot
        .groups
        .iter()
        .find(|group| group.global_id == active.unique_global_id)
        .unwrap();
    assert_eq!(active_row.name, "active");
    assert_eq!(active_row.deleted_at, None);
    let expected_revision: i64 =
        sqlx::query_scalar("SELECT group_revision FROM models_group WHERE group_id = ?")
            .bind(active.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(active_row.revision, expected_revision);
    let deleted_row = snapshot
        .groups
        .iter()
        .find(|group| group.global_id == deleted.unique_global_id)
        .unwrap();
    assert_eq!(deleted_row.name, "deleted");
    assert!(deleted_row.deleted_at.is_some());
    assert_eq!(snapshot.assignments.len(), 3);
    for (id, global, group, trashed) in [
        (
            live_id,
            &live_global,
            Some(active.unique_global_id.as_str()),
            false,
        ),
        (
            trash_id,
            &trash_global,
            Some(active.unique_global_id.as_str()),
            true,
        ),
        (ungrouped_id, &ungrouped_global, None, false),
    ] {
        let assignment = snapshot
            .assignments
            .iter()
            .find(|assignment| &assignment.model_global_id == global)
            .unwrap();
        assert_eq!(assignment.group_global_id.as_deref(), group);
        assert_eq!(assignment.deleted_at.is_some(), trashed);
        let revision: i64 =
            sqlx::query_scalar("SELECT model_assignment_revision FROM models WHERE model_id = ?")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(assignment.revision, revision);
    }
    assert_eq!(snapshot.deletions.len(), 2);
    for global in [&deleted.unique_global_id, &purged.unique_global_id] {
        let record = snapshot
            .deletions
            .iter()
            .find(|deletion| &deletion.group_global_id == global)
            .unwrap();
        assert_ne!(record.operation_id, "");
        assert_ne!(record.deleted_at, "");
        assert_eq!(record.restored_at, None);
        let expected: i64 = sqlx::query_scalar(
            "SELECT group_revision FROM group_deletion_records WHERE operation_id = ?",
        )
        .bind(&record.operation_id)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(record.revision, expected);
    }
    assert!(
        !snapshot
            .assignments
            .iter()
            .any(|assignment| assignment.model_global_id == foreign_global)
    );
    assert!(
        !snapshot
            .deletions
            .iter()
            .any(|deletion| deletion.group_global_id == foreign_group.unique_global_id)
    );
    let empty = group_db::get_group_sync_snapshot(
        &db,
        &User {
            id: -1,
            ..User::default()
        },
    )
    .await
    .unwrap();
    assert!(empty.groups.is_empty() && empty.assignments.is_empty() && empty.deletions.is_empty());

    let restore = common::group_restore_request(&db, &owner, deleted.id).await;
    group_db::restore_groups(&db, &owner, &[restore], 30)
        .await
        .unwrap();
    let restored = group_db::get_group_sync_snapshot(&db, &owner)
        .await
        .unwrap();
    let record = restored
        .deletions
        .iter()
        .find(|deletion| deletion.group_global_id == deleted.unique_global_id)
        .unwrap();
    assert!(record.restored_at.is_some());
}

#[tokio::test]
async fn snapshot_rejects_missing_or_ambiguous_global_ids() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let first = group_db::add_empty_group(&db, &user, "first", None)
        .await
        .unwrap();
    let second = group_db::add_empty_group(&db, &user, "second", None)
        .await
        .unwrap();
    let (model1, global1) = add_model(&db, &user, "first-model").await;
    let (model2, global2) = add_model(&db, &user, "second-model").await;
    for (sql, value, id) in [
        (
            "UPDATE models_group SET group_unique_global_id = ? WHERE group_id = ?",
            "",
            second.id,
        ),
        (
            "UPDATE models_group SET group_unique_global_id = ? WHERE group_id = ?",
            first.unique_global_id.as_str(),
            second.id,
        ),
    ] {
        sqlx::query(sql)
            .bind(value)
            .bind(id)
            .execute(&db)
            .await
            .unwrap();
        assert!(group_db::get_group_sync_snapshot(&db, &user).await.is_err());
    }
    sqlx::query("UPDATE models_group SET group_unique_global_id = ? WHERE group_id = ?")
        .bind(&second.unique_global_id)
        .bind(second.id)
        .execute(&db)
        .await
        .unwrap();
    for value in ["", global1.as_str()] {
        sqlx::query("UPDATE models SET model_unique_global_id = ? WHERE model_id = ?")
            .bind(value)
            .bind(model2)
            .execute(&db)
            .await
            .unwrap();
        assert!(group_db::get_group_sync_snapshot(&db, &user).await.is_err());
    }
    sqlx::query("UPDATE models SET model_unique_global_id = ? WHERE model_id = ?")
        .bind(global2)
        .bind(model2)
        .execute(&db)
        .await
        .unwrap();
    assert!(group_db::get_group_sync_snapshot(&db, &user).await.is_ok());
    let _ = model1;
}

#[tokio::test]
async fn snapshots_never_mix_states_from_concurrent_commits() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "off", None)
        .await
        .unwrap();
    let (model_id, model_global) = add_model(&db, &user, "racing-model").await;
    let writer_db = db.clone();
    let group_id = group.id;
    let writer = tokio::spawn(async move {
        for i in 0..100 {
            let mut tx = writer_db.begin().await.unwrap();
            let on = i % 2 == 0;
            sqlx::query("UPDATE models_group SET group_name = ? WHERE group_id = ?")
                .bind(if on { "on" } else { "off" })
                .bind(group_id)
                .execute(&mut *tx)
                .await
                .unwrap();
            sqlx::query("UPDATE models SET model_group_id = ? WHERE model_id = ?")
                .bind(if on { Some(group_id) } else { None })
                .bind(model_id)
                .execute(&mut *tx)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            tokio::task::yield_now().await;
        }
    });
    for _ in 0..100 {
        let snapshot = group_db::get_group_sync_snapshot(&db, &user).await.unwrap();
        let group_row = snapshot
            .groups
            .iter()
            .find(|item| item.global_id == group.unique_global_id)
            .unwrap();
        let model = snapshot
            .assignments
            .iter()
            .find(|assignment| assignment.model_global_id == model_global)
            .unwrap();
        assert_eq!(group_row.name == "on", model.group_global_id.is_some());
    }
    writer.await.unwrap();
}
