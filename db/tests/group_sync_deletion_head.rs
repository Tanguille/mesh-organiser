//! Snapshot heads identify the latest deletion independently of Trash retention.
mod common;
use db::{db_context::DbContext, group_db, model::user::User, random_hex_32, user_db};

async fn last_delete(db: &DbContext, user: &User, global: &str) -> Option<String> {
    let snapshot = group_db::get_group_sync_snapshot(db, user).await.unwrap();
    let head = snapshot
        .heads
        .iter()
        .find(|head| head.group_global_id == global)
        .unwrap();
    head.last_deletion_operation_id.clone()
}

#[tokio::test]
async fn latest_deletion_event_remains_visible_after_restore_and_purge() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let foreign_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let foreign = user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &owner, "folder", None)
        .await
        .unwrap();
    let foreign_group = group_db::add_empty_group(&db, &foreign, "foreign", None)
        .await
        .unwrap();
    let global = &group.unique_global_id;
    group_db::edit_group(
        &db,
        &foreign,
        foreign_group.id,
        "foreign",
        None,
        Some(global),
    )
    .await
    .unwrap();
    assert_eq!(last_delete(&db, &owner, global).await, None);
    assert_eq!(last_delete(&db, &foreign, global).await, None);
    group_db::delete_group(&db, &foreign, foreign_group.id)
        .await
        .unwrap();
    let foreign_delete = common::group_restore_request(&db, &foreign, foreign_group.id).await;
    group_db::delete_group(&db, &owner, group.id).await.unwrap();
    let first = common::group_restore_request(&db, &owner, group.id).await;
    assert_eq!(
        last_delete(&db, &owner, global).await.as_deref(),
        Some(first.deletion_operation_id.as_str())
    );
    group_db::restore_groups(&db, &owner, std::slice::from_ref(&first), 30)
        .await
        .unwrap();
    assert_eq!(
        last_delete(&db, &owner, global).await.as_deref(),
        Some(first.deletion_operation_id.as_str())
    );
    group_db::delete_group(&db, &owner, group.id).await.unwrap();
    let second = common::group_restore_request(&db, &owner, group.id).await;
    assert_ne!(first.deletion_operation_id, second.deletion_operation_id);
    sqlx::query("UPDATE group_deletion_records SET deleted_at = '2099-01-01T00:00:00Z' WHERE user_id = ? AND group_global_id = ?").bind(owner.id).bind(global).execute(&db).await.unwrap();
    assert_eq!(
        last_delete(&db, &owner, global).await.as_deref(),
        Some(second.deletion_operation_id.as_str())
    );
    assert_eq!(
        last_delete(&db, &foreign, global).await.as_deref(),
        Some(foreign_delete.deletion_operation_id.as_str())
    );
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(&db)
    .await
    .unwrap();
    group_db::purge_trashed_groups(&db, 30).await.unwrap();
    assert_eq!(
        last_delete(&db, &owner, global).await.as_deref(),
        Some(second.deletion_operation_id.as_str())
    );
    assert_eq!(
        last_delete(&db, &foreign, global).await.as_deref(),
        Some(foreign_delete.deletion_operation_id.as_str())
    );
}
