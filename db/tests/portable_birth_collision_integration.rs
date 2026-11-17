//! A retained birth must prevent a different entity from taking its global identity.
mod common;

use db::{
    blob_db, group_db,
    model::{ModelFlags, user::User},
    model_db, random_hex_32, user_db,
};

#[tokio::test]
async fn purged_model_identity_cannot_be_reused_by_different_birth_for_same_owner() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let old_blob = blob_db::add_blob(&db, "old-model", "stl", 1, None)
        .await
        .unwrap();
    let old = model_db::add_model(&db, &owner, "old", old_blob, None, None)
        .await
        .unwrap();
    let old_global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(old)
            .fetch_one(&db)
            .await
            .unwrap();
    model_db::trash_models(&db, &owner, &[old]).await.unwrap();
    sqlx::query("UPDATE models SET model_deleted = '2000-01-01T00:00:00Z' WHERE model_id = ?")
        .bind(old)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(model_db::purge_trashed_models(&db, 30).await.unwrap(), 1);
    let new_blob = blob_db::add_blob(&db, "new-model", "stl", 1, None)
        .await
        .unwrap();
    let current = model_db::add_model(&db, &owner, "current", new_blob, None, None)
        .await
        .unwrap();
    let current_global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(current)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(
        model_db::edit_model(
            &db,
            &owner,
            current,
            "current",
            None,
            None,
            ModelFlags::empty(),
            None,
            Some(&old_global)
        )
        .await
        .is_err()
    );
    let after: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(current)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(after, current_global);
    let records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entity_births WHERE user_id = ? AND entity_kind = 'model' AND global_id = ?").bind(owner.id).bind(&old_global).fetch_one(&db).await.unwrap();
    assert_eq!(records, 1);
    let foreign_blob = blob_db::add_blob(&db, "foreign-model", "stl", 1, None)
        .await
        .unwrap();
    let foreign = model_db::add_model(&db, &other, "foreign", foreign_blob, None, None)
        .await
        .unwrap();
    model_db::edit_model(
        &db,
        &other,
        foreign,
        "foreign",
        None,
        None,
        ModelFlags::empty(),
        None,
        Some(&old_global),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn purged_group_identity_cannot_be_reused_by_different_birth_for_same_owner() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let old = group_db::add_empty_group(&db, &owner, "old", None)
        .await
        .unwrap();
    // A historical cleanup can remove a group without a causal deletion event.
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(old.id)
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(group_db::purge_trashed_groups(&db, 30).await.unwrap(), 1);
    let current = group_db::add_empty_group(&db, &owner, "current", None)
        .await
        .unwrap();
    assert!(
        group_db::edit_group(
            &db,
            &owner,
            current.id,
            "current",
            None,
            Some(&old.unique_global_id)
        )
        .await
        .is_err()
    );
    let after: String =
        sqlx::query_scalar("SELECT group_unique_global_id FROM models_group WHERE group_id = ?")
            .bind(current.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(after, current.unique_global_id);
    let records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entity_births WHERE user_id = ? AND entity_kind = 'group' AND global_id = ?").bind(owner.id).bind(&old.unique_global_id).fetch_one(&db).await.unwrap();
    assert_eq!(records, 1);
    let foreign = group_db::add_empty_group(&db, &other, "foreign", None)
        .await
        .unwrap();
    group_db::edit_group(
        &db,
        &other,
        foreign.id,
        "foreign",
        None,
        Some(&old.unique_global_id),
    )
    .await
    .unwrap();
}
