//! A stale Trash selection must not restore a later deletion of the same group.
use db::{
    group_db::{self, GroupRestoreRequest},
    model::user::User,
    random_hex_32, user_db,
};

mod common;

async fn request(db: &db::db_context::DbContext, user: &User, id: i64) -> GroupRestoreRequest {
    let items = group_db::get_trashed_groups(db, user, 30).await.unwrap();
    let item = items.iter().find(|item| item.id == id);
    assert!(item.is_some(), "group {id} must remain recoverable");
    let item = item.unwrap();
    GroupRestoreRequest {
        id,
        group_global_id: item.group_global_id.clone(),
        deletion_operation_id: item.deletion_operation_id.clone().unwrap(),
    }
}

#[tokio::test]
async fn stale_restore_cannot_restore_a_later_deletion_even_in_the_same_second() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2099-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(&db)
    .await
    .unwrap();
    let first = request(&db, &user, group.id).await;
    assert_eq!(first.group_global_id, group.unique_global_id);
    assert_ne!(first.deletion_operation_id, "");
    group_db::restore_groups(&db, &user, std::slice::from_ref(&first), 30)
        .await
        .unwrap();
    group_db::restore_groups(&db, &user, std::slice::from_ref(&first), 30)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2099-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query("UPDATE group_deletion_records SET deleted_at = '2099-01-01T00:00:00Z' WHERE user_id = ? AND group_global_id = ?").bind(user.id).bind(&group.unique_global_id).execute(&db).await.unwrap();
    let second = request(&db, &user, group.id).await;
    assert_ne!(first.deletion_operation_id, second.deletion_operation_id);
    assert!(
        group_db::restore_groups(&db, &user, std::slice::from_ref(&first), 30)
            .await
            .is_err()
    );
    assert_eq!(
        request(&db, &user, group.id).await.deletion_operation_id,
        second.deletion_operation_id
    );
    group_db::restore_groups(&db, &user, std::slice::from_ref(&second), 30)
        .await
        .unwrap();
    assert!(
        group_db::restore_groups(&db, &user, &[first], 30)
            .await
            .is_err(),
        "old retries remain stale after the newer deletion is restored"
    );
}

#[tokio::test]
async fn restore_batch_rejects_foreign_or_expired_entries_without_restoring_valid_ones() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let foreign_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let foreign = user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let valid = group_db::add_empty_group(&db, &owner, "valid", None)
        .await
        .unwrap();
    let expired = group_db::add_empty_group(&db, &owner, "expired", None)
        .await
        .unwrap();
    let other = group_db::add_empty_group(&db, &foreign, "foreign", None)
        .await
        .unwrap();
    group_db::delete_group(&db, &owner, valid.id).await.unwrap();
    group_db::delete_group(&db, &owner, expired.id)
        .await
        .unwrap();
    group_db::delete_group(&db, &foreign, other.id)
        .await
        .unwrap();
    let good = request(&db, &owner, valid.id).await;
    let too_old = request(&db, &owner, expired.id).await;
    let someone_else = request(&db, &foreign, other.id).await;
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(expired.id)
    .execute(&db)
    .await
    .unwrap();
    let visible = group_db::get_trashed_groups(&db, &owner, 30).await.unwrap();
    assert!(visible.iter().any(|item| item.id == valid.id));
    assert!(
        !visible.iter().any(|item| item.id == expired.id),
        "expired folders cannot be restored and must not be listed"
    );
    assert!(
        group_db::restore_groups(&db, &owner, &[good.clone(), someone_else], 30)
            .await
            .is_err()
    );
    assert!(
        group_db::restore_groups(&db, &owner, &[good.clone(), too_old], 30)
            .await
            .is_err()
    );
    assert_eq!(
        request(&db, &owner, valid.id).await.deletion_operation_id,
        good.deletion_operation_id
    );
    assert_ne!(
        request(&db, &foreign, other.id).await.deletion_operation_id,
        ""
    );
}

#[tokio::test]
async fn restore_rejects_wrong_group_identity_or_operation() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let good = request(&db, &user, group.id).await;
    let mut wrong_global = good.clone();
    wrong_global.group_global_id = random_hex_32();
    let mut wrong_operation = good.clone();
    wrong_operation.deletion_operation_id = random_hex_32();
    for invalid in [wrong_global, wrong_operation] {
        assert!(
            group_db::restore_groups(&db, &user, &[invalid], 30)
                .await
                .is_err()
        );
    }
    assert_eq!(
        request(&db, &user, group.id).await.deletion_operation_id,
        good.deletion_operation_id
    );
}

#[tokio::test]
async fn migration_recovers_single_unresolved_deletion_among_historical_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|migration| migration.version < 20_261_112_180_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let group = common::legacy_group(&db, &user, "historical").await;
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2099-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(&db)
    .await
    .unwrap();
    let old_id = random_hex_32();
    let current_id = random_hex_32();
    for (operation_id, restored) in [(&old_id, Some("2099-01-01T00:00:00Z")), (&current_id, None)] {
        sqlx::query("INSERT INTO group_deletion_records (user_id, group_global_id, operation_id, deleted_at, restored_at, group_revision) VALUES (?, ?, ?, '2099-01-01T00:00:00Z', ?, 0)")
            .bind(user.id).bind(&group.unique_global_id).bind(operation_id).bind(restored).execute(&db).await.unwrap();
    }
    db.close().await;
    let upgraded = db::db_context::setup_db(&path, &dir.path().join("backups")).await;
    let listing = group_db::get_trashed_groups(&upgraded, &user, 30).await;
    assert!(
        listing.is_ok(),
        "a uniquely unresolved historical deletion must remain listable"
    );
    let item = listing
        .unwrap()
        .into_iter()
        .find(|item| item.id == group.id);
    assert!(
        item.is_some(),
        "a uniquely unresolved deletion must remain recoverable"
    );
    let item = item.unwrap();
    assert_eq!(
        item.deletion_operation_id.as_deref(),
        Some(current_id.as_str())
    );
    let restore = GroupRestoreRequest {
        id: item.id,
        group_global_id: item.group_global_id,
        deletion_operation_id: item.deletion_operation_id.unwrap(),
    };
    assert!(
        group_db::restore_groups(&upgraded, &user, &[restore], 30)
            .await
            .is_ok()
    );
}
