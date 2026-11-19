//! Existing group deletion keeps live and trashed model rows and respects ownership.

use db::{db_context::DbContext, group_db, model::user::User, model_db, user_db};

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

#[tokio::test]
async fn deleting_group_ungroups_live_and_trashed_models_without_deleting_rows() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "parts", None)
        .await
        .unwrap();
    let live = add_model(&db, &user, "live-part").await;
    let trashed = add_model(&db, &user, "trashed-part").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![live, trashed], None)
        .await
        .unwrap();
    model_db::trash_models(&db, &user, &[trashed])
        .await
        .unwrap();

    group_db::delete_group(&db, &user, group.id).await.unwrap();

    assert_eq!(model_group(&db, live).await, None);
    assert_eq!(model_group(&db, trashed).await, None);
    let deleted: Option<String> =
        sqlx::query_scalar("SELECT model_deleted FROM models WHERE model_id = ?")
            .bind(trashed)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(deleted.is_some(), "the trashed model must remain in Trash");
}

#[tokio::test]
async fn deleting_another_users_group_leaves_its_models_and_group_unchanged() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.com", &db::random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let foreign_group = group_db::add_empty_group(&db, &other, "foreign", None)
        .await
        .unwrap();
    let foreign_model = add_model(&db, &other, "foreign-part").await;
    group_db::set_group_id_on_models(
        &db,
        &other,
        Some(foreign_group.id),
        vec![foreign_model],
        None,
    )
    .await
    .unwrap();

    group_db::delete_group(&db, &user, foreign_group.id)
        .await
        .unwrap();

    assert_eq!(
        model_group(&db, foreign_model).await,
        Some(foreign_group.id)
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models_group WHERE group_id = ?")
        .bind(foreign_group.id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 1);
}
