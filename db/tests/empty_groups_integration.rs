//! Empty model groups remain usable destinations without exposing another user's groups.

use db::{
    blob_db,
    db_context::DbContext,
    group_db::{self, GroupFilterOptions, GroupOrderBy},
    model::{blob::FileType, user::User},
    model_db, random_hex_32, resource_db, user_db,
};

mod common;
use common::test_db;

async fn add_model(db: &DbContext, user: &User, name: &str) -> i64 {
    let blob_id = blob_db::add_blob(db, name, "stl", 1, None).await.unwrap();
    model_db::add_model(db, user, name, blob_id, None, None)
        .await
        .unwrap()
}

async fn other_user(db: &DbContext) -> User {
    let id = user_db::add_user(db, "other", "other@example.com", &random_hex_32())
        .await
        .unwrap();
    user_db::get_user_by_id(db, id).await.unwrap().unwrap()
}

#[tokio::test]
async fn empty_groups_are_visible_only_to_their_owner_and_can_be_loaded_by_id() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other = other_user(&db).await;
    let own = group_db::add_empty_group(&db, &user, "My folder", None)
        .await
        .unwrap();
    let foreign = group_db::add_empty_group(&db, &other, "Other folder", None)
        .await
        .unwrap();

    let all = group_db::get_groups(&db, &user, GroupFilterOptions::default())
        .await
        .unwrap();
    assert_eq!(all.items.len(), 1);
    assert_eq!(all.items[0].meta.id, own.id);
    assert!(all.items[0].models.is_empty());
    let own_by_id = group_db::get_groups(
        &db,
        &user,
        GroupFilterOptions {
            group_ids: Some(vec![own.id]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        own_by_id
            .items
            .iter()
            .map(|group| group.meta.id)
            .collect::<Vec<_>>(),
        vec![own.id]
    );
    let foreign_by_id = group_db::get_groups(
        &db,
        &user,
        GroupFilterOptions {
            group_ids: Some(vec![foreign.id]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(foreign_by_id.items.is_empty());
    let none_requested = group_db::get_groups(
        &db,
        &user,
        GroupFilterOptions {
            group_ids: Some(vec![]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(none_requested.items.is_empty());
}

#[tokio::test]
async fn empty_groups_take_their_sorted_place_before_pagination() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let empty = group_db::add_empty_group(&db, &user, "A empty", None)
        .await
        .unwrap();
    let populated = group_db::add_empty_group(&db, &user, "B populated", None)
        .await
        .unwrap();
    let model_id = add_model(&db, &user, "part").await;
    group_db::set_group_id_on_models(&db, &user, Some(populated.id), vec![model_id], None)
        .await
        .unwrap();
    for (page, id) in [(1, empty.id), (2, populated.id)] {
        let groups = group_db::get_groups(
            &db,
            &user,
            GroupFilterOptions {
                order_by: Some(GroupOrderBy::NameAsc),
                page,
                page_size: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            groups
                .items
                .iter()
                .map(|group| group.meta.id)
                .collect::<Vec<_>>(),
            vec![id]
        );
    }
}

#[tokio::test]
async fn empty_group_name_search_and_model_filters_are_respected() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let empty = group_db::add_empty_group(&db, &user, "Spare bolts", None)
        .await
        .unwrap();
    let ungrouped = add_model(&db, &user, "model").await;
    let search = group_db::get_groups(
        &db,
        &user,
        GroupFilterOptions {
            text_search: Some("bolts".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        search
            .items
            .iter()
            .map(|group| group.meta.id)
            .collect::<Vec<_>>(),
        vec![empty.id]
    );
    let nonmatch = group_db::get_groups(
        &db,
        &user,
        GroupFilterOptions {
            text_search: Some("unrelated".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(nonmatch.items.is_empty());
    for filter in [
        GroupFilterOptions {
            model_ids: Some(vec![ungrouped]),
            ..Default::default()
        },
        GroupFilterOptions {
            label_ids: Some(vec![999]),
            ..Default::default()
        },
        GroupFilterOptions {
            file_types: Some(vec![FileType::Stl]),
            ..Default::default()
        },
    ] {
        assert!(
            group_db::get_groups(&db, &user, filter)
                .await
                .unwrap()
                .items
                .is_empty()
        );
    }
}

#[tokio::test]
async fn group_count_matches_visible_groups_including_empty_but_not_trashed_or_foreign() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other = other_user(&db).await;
    group_db::add_empty_group(&db, &user, "empty", None)
        .await
        .unwrap();
    group_db::add_empty_group(&db, &other, "foreign", None)
        .await
        .unwrap();
    let trashed_group = group_db::add_empty_group(&db, &user, "trashed", None)
        .await
        .unwrap();
    let trashed = add_model(&db, &user, "trashed-model").await;
    group_db::set_group_id_on_models(&db, &user, Some(trashed_group.id), vec![trashed], None)
        .await
        .unwrap();
    model_db::trash_models(&db, &user, &[trashed])
        .await
        .unwrap();
    add_model(&db, &user, "ungrouped").await;
    assert_eq!(
        group_db::get_group_count(&db, &user, false).await.unwrap(),
        1
    );
    assert_eq!(
        group_db::get_group_count(&db, &user, true).await.unwrap(),
        2
    );
    let groups = group_db::get_groups(&db, &user, GroupFilterOptions::default())
        .await
        .unwrap();
    assert_eq!(groups.items.len(), 1);
    assert_eq!(groups.items[0].meta.name, "empty");
}

#[tokio::test]
async fn empty_group_keeps_its_resource_metadata() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let resource = resource_db::add_resource(&db, &user, "project", None)
        .await
        .unwrap();
    sqlx::query("UPDATE models_group SET group_resource_id = ? WHERE group_id = ?")
        .bind(resource.id)
        .bind(group.id)
        .execute(&db)
        .await
        .unwrap();
    let groups = group_db::get_groups(
        &db,
        &user,
        GroupFilterOptions {
            group_ids: Some(vec![group.id]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(groups.items.len(), 1);
    assert_eq!(
        groups.items[0].resource.as_ref().map(|r| r.id),
        Some(resource.id)
    );
}
