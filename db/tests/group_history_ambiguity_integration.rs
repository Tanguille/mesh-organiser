//! Ambiguous same-owner identities must never merge causal histories.

use db::{db_context::DbContext, group_db, model::user::User, model_db};

mod common;

async fn add_model(db: &DbContext, user: &User, name: &str) -> i64 {
    let blob = db::blob_db::add_blob(db, name, "stl", 1, None)
        .await
        .unwrap();
    model_db::add_model(db, user, name, blob, None, None)
        .await
        .unwrap()
}

async fn event_counts(db: &DbContext) -> (i64, i64, i64) {
    let group = sqlx::query_scalar("SELECT COUNT(*) FROM group_causal_events")
        .fetch_one(db)
        .await
        .unwrap();
    let model = sqlx::query_scalar("SELECT COUNT(*) FROM membership_causal_events")
        .fetch_one(db)
        .await
        .unwrap();
    let deletion = sqlx::query_scalar("SELECT COUNT(*) FROM group_deletion_records")
        .fetch_one(db)
        .await
        .unwrap();
    (group, model, deletion)
}

#[tokio::test]
async fn delete_rejects_ambiguous_group_identity_without_changes() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let first_group = group_db::add_empty_group(&db, &user, "A", None)
        .await
        .unwrap();
    let second_group = group_db::add_empty_group(&db, &user, "B", None)
        .await
        .unwrap();
    let id = add_model(&db, &user, "member").await;
    group_db::set_group_id_on_models(&db, &user, Some(first_group.id), vec![id], None)
        .await
        .unwrap();
    sqlx::query("UPDATE models_group SET group_unique_global_id = ? WHERE group_id = ?")
        .bind(&first_group.unique_global_id)
        .bind(second_group.id)
        .execute(&db)
        .await
        .unwrap();
    let before = event_counts(&db).await;
    assert!(
        group_db::delete_group(&db, &user, first_group.id)
            .await
            .is_err()
    );
    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(assigned, Some(first_group.id));
    let deleted: Option<String> =
        sqlx::query_scalar("SELECT group_deleted_at FROM models_group WHERE group_id = ?")
            .bind(first_group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(deleted, None);
    assert_eq!(event_counts(&db).await, before);
}

#[tokio::test]
async fn delete_rejects_ambiguous_member_identity_without_changes() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    let first = add_model(&db, &user, "first").await;
    let second = add_model(&db, &user, "second").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![first], None)
        .await
        .unwrap();
    let global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(first)
            .fetch_one(&db)
            .await
            .unwrap();
    sqlx::query("UPDATE models SET model_unique_global_id = ? WHERE model_id = ?")
        .bind(global)
        .bind(second)
        .execute(&db)
        .await
        .unwrap();
    let before = event_counts(&db).await;
    assert!(group_db::delete_group(&db, &user, group.id).await.is_err());
    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
            .bind(first)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(assigned, Some(group.id));
    let deleted: Option<String> =
        sqlx::query_scalar("SELECT group_deleted_at FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(deleted, None);
    assert_eq!(event_counts(&db).await, before);
}

#[tokio::test]
async fn rename_rejects_ambiguous_group_identity_without_changes() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let first_group = group_db::add_empty_group(&db, &user, "A", None)
        .await
        .unwrap();
    let second_group = group_db::add_empty_group(&db, &user, "B", None)
        .await
        .unwrap();
    sqlx::query("UPDATE models_group SET group_unique_global_id = ? WHERE group_id = ?")
        .bind(&first_group.unique_global_id)
        .bind(second_group.id)
        .execute(&db)
        .await
        .unwrap();
    let before = event_counts(&db).await;
    assert!(
        group_db::edit_group(&db, &user, first_group.id, "renamed", None, None)
            .await
            .is_err()
    );
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id = ?")
        .bind(first_group.id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(name, "A");
    assert_eq!(event_counts(&db).await, before);
}

#[tokio::test]
async fn rekey_cannot_merge_two_distinct_group_histories() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let first_group = group_db::add_empty_group(&db, &user, "A", None)
        .await
        .unwrap();
    let second_group = group_db::add_empty_group(&db, &user, "B", None)
        .await
        .unwrap();
    group_db::edit_group(&db, &user, first_group.id, "A renamed", None, None)
        .await
        .unwrap();
    group_db::edit_group(&db, &user, second_group.id, "B renamed", None, None)
        .await
        .unwrap();
    let before = event_counts(&db).await;
    assert!(
        group_db::edit_group(
            &db,
            &user,
            first_group.id,
            "A renamed again",
            None,
            Some(&second_group.unique_global_id)
        )
        .await
        .is_err()
    );
    let (name, global): (String, String) = sqlx::query_as(
        "SELECT group_name, group_unique_global_id FROM models_group WHERE group_id = ?",
    )
    .bind(first_group.id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(name, "A renamed");
    assert_eq!(global, first_group.unique_global_id);
    assert_eq!(event_counts(&db).await, before);
}

#[tokio::test]
async fn group_rekey_after_history_is_rejected_but_regular_rename_works() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "before", None)
        .await
        .unwrap();
    group_db::edit_group(&db, &user, group.id, "first rename", None, None)
        .await
        .unwrap();
    let before = event_counts(&db).await;
    let new_global = db::random_hex_32();
    assert!(
        group_db::edit_group(
            &db,
            &user,
            group.id,
            "first rename",
            None,
            Some(&new_global)
        )
        .await
        .is_err()
    );
    let (name, global): (String, String) = sqlx::query_as(
        "SELECT group_name, group_unique_global_id FROM models_group WHERE group_id = ?",
    )
    .bind(group.id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(name, "first rename");
    assert_eq!(global, group.unique_global_id);
    assert_eq!(event_counts(&db).await, before);
    group_db::edit_group(&db, &user, group.id, "second rename", None, None)
        .await
        .unwrap();
    let history = db::group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(history.events.len(), 2);
    assert_eq!(
        history.events[1].parents,
        vec![history.events[0].id.clone()]
    );
}

#[tokio::test]
async fn model_rekey_after_membership_history_is_rejected_without_losing_history() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    let id = add_model(&db, &user, "model").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![id], None)
        .await
        .unwrap();
    let (global, incarnation): (String, String) = sqlx::query_as(
        "SELECT model_unique_global_id, model_incarnation FROM models WHERE model_id = ?",
    )
    .bind(id)
    .fetch_one(&db)
    .await
    .unwrap();
    let before = event_counts(&db).await;
    let new_global = db::random_hex_32();
    assert!(
        model_db::edit_model(
            &db,
            &user,
            id,
            "model",
            None,
            None,
            db::model::ModelFlags::empty(),
            None,
            Some(&new_global)
        )
        .await
        .is_err()
    );
    let after: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(after, global);
    assert_eq!(event_counts(&db).await, before);
    let history = db::group_history::get_membership_history(&db, &user, &global, &incarnation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(history.events.len(), 1);
}

#[tokio::test]
async fn rekey_rejects_existing_target_even_without_prior_events() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let first_group = group_db::add_empty_group(&db, &user, "A", None)
        .await
        .unwrap();
    let second_group = group_db::add_empty_group(&db, &user, "B", None)
        .await
        .unwrap();
    let before = event_counts(&db).await;
    assert!(
        group_db::edit_group(
            &db,
            &user,
            first_group.id,
            "A",
            None,
            Some(&second_group.unique_global_id)
        )
        .await
        .is_err()
    );
    let global: String =
        sqlx::query_scalar("SELECT group_unique_global_id FROM models_group WHERE group_id = ?")
            .bind(first_group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(global, first_group.unique_global_id);
    assert_eq!(event_counts(&db).await, before);
}

#[tokio::test]
async fn model_rekey_rejects_same_owner_collision_but_allows_other_owner_identity() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let other_id = db::user_db::add_user(&db, "other", "other@example.test", &db::random_hex_32())
        .await
        .unwrap();
    let other = db::user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let first_model = add_model(&db, &user, "A").await;
    let second_model = add_model(&db, &user, "B").await;
    let third_model = add_model(&db, &other, "C").await;
    let get_global = |id| {
        sqlx::query_scalar::<_, String>(
            "SELECT model_unique_global_id FROM models WHERE model_id = ?",
        )
        .bind(id)
        .fetch_one(&db)
    };
    let a_global = get_global(first_model).await.unwrap();
    let b_global = get_global(second_model).await.unwrap();
    let c_global = get_global(third_model).await.unwrap();
    let before = event_counts(&db).await;
    assert!(
        model_db::edit_model(
            &db,
            &user,
            first_model,
            "A",
            None,
            None,
            db::model::ModelFlags::empty(),
            None,
            Some(&b_global)
        )
        .await
        .is_err()
    );
    assert_eq!(get_global(first_model).await.unwrap(), a_global);
    assert_eq!(event_counts(&db).await, before);
    model_db::edit_model(
        &db,
        &user,
        first_model,
        "A",
        None,
        None,
        db::model::ModelFlags::empty(),
        None,
        Some(&c_global),
    )
    .await
    .unwrap();
    assert_eq!(get_global(first_model).await.unwrap(), c_global);
}
