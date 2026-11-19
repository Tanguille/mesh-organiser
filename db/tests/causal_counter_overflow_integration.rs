//! Exhausted causal counters must fail before any domain or history state commits.

use db::{blob_db, db_context::DbContext, group_db, model::user::User, model_db, random_hex_32};

mod common;

async fn exhaust(db: &DbContext) {
    sqlx::query("UPDATE group_causal_replica SET counter = ? WHERE singleton = 1")
        .bind(i64::MAX)
        .execute(db)
        .await
        .unwrap();
}

async fn counts(db: &DbContext) -> (i64, i64, i64, i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group), (SELECT COUNT(*) FROM models), (SELECT COUNT(*) FROM group_causal_events), (SELECT COUNT(*) FROM membership_causal_events), (SELECT COUNT(*) FROM entity_births), (SELECT COUNT(*) FROM local_creation_records)")
        .fetch_one(db).await.unwrap()
}

async fn counter(db: &DbContext) -> i64 {
    sqlx::query_scalar("SELECT counter FROM group_causal_replica WHERE singleton = 1")
        .fetch_one(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn group_creation_fails_atomically_when_counter_is_exhausted() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    exhaust(&db).await;
    let before = counts(&db).await;
    assert!(
        group_db::add_empty_group(&db, &user, "must not exist", None)
            .await
            .is_err()
    );
    assert_eq!(counts(&db).await, before);
    assert_eq!(counter(&db).await, i64::MAX);
}

#[tokio::test]
async fn model_creation_fails_atomically_when_counter_is_exhausted() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    exhaust(&db).await;
    let before = counts(&db).await;
    let highwater: i64 =
        sqlx::query_scalar("SELECT highwater FROM object_id_highwater WHERE object_kind = 'model'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(
        model_db::add_model(&db, &user, "must not exist", blob, None, None)
            .await
            .is_err()
    );
    assert_eq!(counts(&db).await, before);
    assert_eq!(counter(&db).await, i64::MAX);
    let after: i64 =
        sqlx::query_scalar("SELECT highwater FROM object_id_highwater WHERE object_kind = 'model'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(after, highwater);
}

#[tokio::test]
async fn group_rename_fails_atomically_when_counter_is_exhausted() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "before", None)
        .await
        .unwrap();
    exhaust(&db).await;
    let before = counts(&db).await;
    let revision: i64 =
        sqlx::query_scalar("SELECT group_revision FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let head: i64 = sqlx::query_scalar(
        "SELECT head FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&group.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(
        group_db::edit_group(&db, &user, group.id, "after", None, None)
            .await
            .is_err()
    );
    let (name, after_revision): (String, i64) =
        sqlx::query_as("SELECT group_name, group_revision FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let after_head: i64 = sqlx::query_scalar(
        "SELECT head FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&group.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(name, "before");
    assert_eq!(after_revision, revision);
    assert_eq!(after_head, head);
    assert_eq!(counts(&db).await, before);
    assert_eq!(counter(&db).await, i64::MAX);
}

#[tokio::test]
async fn model_move_fails_atomically_when_counter_is_exhausted() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "destination", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let id = model_db::add_model(&db, &user, "model", blob, None, None)
        .await
        .unwrap();
    exhaust(&db).await;
    let before = counts(&db).await;
    let (group_revision, head): (i64, i64) = sqlx::query_as("SELECT g.group_revision, h.head FROM models_group g JOIN group_sync_heads h ON h.user_id = g.group_user_id AND h.group_global_id = g.group_unique_global_id WHERE g.group_id = ?").bind(group.id).fetch_one(&db).await.unwrap();
    assert!(
        group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![id], None)
            .await
            .is_err()
    );
    let after: (Option<i64>, i64) = sqlx::query_as(
        "SELECT model_group_id, model_assignment_revision FROM models WHERE model_id = ?",
    )
    .bind(id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(after, (None, 0));
    let group_after: (i64, i64) = sqlx::query_as("SELECT g.group_revision, h.head FROM models_group g JOIN group_sync_heads h ON h.user_id = g.group_user_id AND h.group_global_id = g.group_unique_global_id WHERE g.group_id = ?").bind(group.id).fetch_one(&db).await.unwrap();
    assert_eq!(group_after, (group_revision, head));
    assert_eq!(counts(&db).await, before);
    assert_eq!(counter(&db).await, i64::MAX);
}

#[tokio::test]
async fn move_rolls_back_first_event_when_second_event_exhausts_counter() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "destination", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let id = model_db::add_model(&db, &user, "model", blob, None, None)
        .await
        .unwrap();
    sqlx::query("UPDATE group_causal_replica SET counter = ? WHERE singleton = 1")
        .bind(i64::MAX - 1)
        .execute(&db)
        .await
        .unwrap();
    let before = counts(&db).await;
    assert!(
        group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![id], None)
            .await
            .is_err()
    );
    let after: (Option<i64>, i64) = sqlx::query_as(
        "SELECT model_group_id, model_assignment_revision FROM models WHERE model_id = ?",
    )
    .bind(id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(after, (None, 0));
    assert_eq!(counts(&db).await, before);
    assert_eq!(counter(&db).await, i64::MAX - 1);
}
