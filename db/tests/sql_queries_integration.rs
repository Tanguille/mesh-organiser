//! Integration tests for dynamic SQL query paths (IN clauses, batch inserts).

use db::{
    blob_db, db_context, group_db, label_db, model::user::User, model_db,
};
use tempfile::tempdir;

async fn test_db() -> (tempfile::TempDir, db_context::DbContext) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("db.sqlite");
    let backup_dir = dir.path().join("backups");
    std::fs::create_dir_all(&backup_dir).unwrap();
    let db = db_context::setup_db(&db_path, &backup_dir).await;
    (dir, db)
}

#[tokio::test]
async fn get_blobs_via_ids_returns_matching_blobs() {
    let (_dir, db) = test_db().await;

    let first_id = blob_db::add_blob(&db, "aa", "stl", 10, None).await.unwrap();
    let second_id = blob_db::add_blob(&db, "bb", "stl", 20, None).await.unwrap();
    blob_db::add_blob(&db, "cc", "stl", 30, None).await.unwrap();

    let blobs = blob_db::get_blobs_via_ids(&db, vec![first_id, second_id])
        .await
        .unwrap();

    assert_eq!(blobs.len(), 2);
    let ids: Vec<i64> = blobs.iter().map(|blob| blob.id).collect();
    assert!(ids.contains(&first_id));
    assert!(ids.contains(&second_id));
}

#[tokio::test]
async fn get_blobs_via_ids_empty_input_returns_empty_vec() {
    let (_dir, db) = test_db().await;

    let blobs = blob_db::get_blobs_via_ids(&db, vec![]).await.unwrap();

    assert!(blobs.is_empty());
}

#[tokio::test]
async fn get_models_via_ids_empty_input_returns_empty_even_when_models_exist() {
    let (_dir, db) = test_db().await;
    let user = User::default();

    let blob_id = blob_db::add_blob(&db, "one", "stl", 1, None).await.unwrap();
    model_db::add_model(&db, &user, "one", blob_id, None, None)
        .await
        .unwrap();
    model_db::add_model(&db, &user, "two", blob_id, None, None)
        .await
        .unwrap();

    let models = model_db::get_models_via_ids(&db, &user, vec![]).await.unwrap();

    assert!(models.is_empty());
}

#[tokio::test]
async fn get_models_via_ids_returns_only_requested_rows_and_valid_sql() {
    let (_dir, db) = test_db().await;
    let user = User::default();

    let blob_id = blob_db::add_blob(&db, "one", "stl", 1, None).await.unwrap();
    let requested_model_id = model_db::add_model(&db, &user, "one", blob_id, None, None)
        .await
        .unwrap();
    model_db::add_model(&db, &user, "two", blob_id, None, None)
        .await
        .unwrap();

    let models = model_db::get_models_via_ids(&db, &user, vec![requested_model_id])
        .await
        .unwrap();

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, requested_model_id);
}

#[tokio::test]
async fn get_groups_filtered_by_ungrouped_model_do_not_expand_to_all_models() {
    let (_dir, db) = test_db().await;
    let user = User::default();

    let blob_id = blob_db::add_blob(&db, "one", "stl", 1, None).await.unwrap();
    let ungrouped_model_id = model_db::add_model(&db, &user, "ungrouped", blob_id, None, None)
        .await
        .unwrap();
    let grouped_model_id = model_db::add_model(&db, &user, "grouped", blob_id, None, None)
        .await
        .unwrap();
    let group_id = group_db::add_empty_group(&db, &user, "group", None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(group_id), vec![grouped_model_id], None)
        .await
        .unwrap();

    let groups = group_db::get_groups(
        &db,
        &user,
        group_db::GroupFilterOptions {
            model_ids: Some(vec![ungrouped_model_id]),
            include_ungrouped_models: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(groups.items.len(), 1);
    assert_eq!(groups.items[0].models.len(), 1);
    assert_eq!(groups.items[0].models[0].id, ungrouped_model_id);
}

#[tokio::test]
async fn delete_models_removes_only_requested_ids() {
    let (_dir, db) = test_db().await;
    let user = User::default();

    let blob_id = blob_db::add_blob(&db, "deadbeef", "stl", 4, None)
        .await
        .unwrap();
    let first_id = model_db::add_model(&db, &user, "one", blob_id, None, None)
        .await
        .unwrap();
    let second_id = model_db::add_model(&db, &user, "two", blob_id, None, None)
        .await
        .unwrap();

    model_db::delete_models(&db, &user, &[first_id]).await.unwrap();

    let remaining = model_db::get_models_via_ids(&db, &user, vec![first_id, second_id])
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, second_id);
}

#[tokio::test]
async fn add_labels_on_models_batch_insert_links_rows() {
    let (_dir, db) = test_db().await;
    let user = User::default();

    let label_id = label_db::add_label(&db, &user, "batch", 1, None)
        .await
        .unwrap();
    let blob_id = blob_db::add_blob(&db, "label-test", "stl", 1, None)
        .await
        .unwrap();
    let model_id = model_db::add_model(&db, &user, "labeled", blob_id, None, None)
        .await
        .unwrap();

    label_db::add_labels_on_models(&db, &user, &[label_id], &[model_id], None)
        .await
        .unwrap();

    let models = model_db::get_models_via_ids(&db, &user, vec![model_id])
        .await
        .unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].labels.len(), 1);
    assert_eq!(models[0].labels[0].id, label_id);
}
