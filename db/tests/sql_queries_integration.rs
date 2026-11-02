//! Integration tests for dynamic SQL query paths (IN clauses, batch inserts).

use db::{
    blob_db, db_context, group_db, label_db,
    model::{blob::FileType, user::User},
    model_db,
};

use common::test_db;

mod common;

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

    let models = model_db::get_models_via_ids(&db, &user, vec![])
        .await
        .unwrap();

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
async fn get_models_via_ids_returns_every_requested_model_past_the_bind_limit_in_model_id_order() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    // Past a page (1000) and past SQLite's 32766 bound variables, so the ids need two queries.
    let model_ids = add_tied_models(&db, &user, 33_000).await;
    // Reversed and repeated, yet each model comes back once in model_id order, as one query
    // over every id would return it.
    let requested: Vec<i64> = model_ids.iter().rev().chain(&model_ids).copied().collect();

    let models = model_db::get_models_via_ids(&db, &user, requested)
        .await
        .unwrap();

    let listed_ids: Vec<i64> = models.iter().map(|model| model.id).collect();
    assert_eq!(listed_ids, model_ids);
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
        .unwrap()
        .id;
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
async fn add_labels_on_models_batch_insert_links_rows() {
    let (_dir, db) = test_db().await;
    let user = User::default();

    let label_id = label_db::add_label(&db, &user, "batch", 1, None)
        .await
        .unwrap()
        .id;
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

#[tokio::test]
async fn get_groups_default_filter_options_does_not_panic() {
    // Regression: `GroupFilterOptions::default()` used to produce `page: 0`, which made
    // `get_groups` underflow on `page - 1` and panic. The fix gives `Default` sensible
    // pagination values (`page: 1`, `page_size: MAX_PAGE_SIZE`) and clamps zeros inside
    // the function, so a caller that builds the struct manually with `0` is also safe.
    let (_dir, db) = test_db().await;
    let user = User::default();

    let default_opts = group_db::GroupFilterOptions::default();
    assert_eq!(default_opts.page, 1, "Default page must be 1-based");
    assert!(default_opts.page_size > 0, "Default page_size must be > 0");

    let _ = group_db::get_groups(&db, &user, group_db::GroupFilterOptions::default())
        .await
        .expect("default filter options should not panic");

    let zero_opts = group_db::GroupFilterOptions {
        page: 0,
        page_size: 0,
        ..group_db::GroupFilterOptions::default()
    };
    let _ = group_db::get_groups(&db, &user, zero_opts)
        .await
        .expect("zero page/page_size must be clamped, not panic");

    // The Tauri command passes `page` through unchecked, so `page * page_size` must not overflow.
    let last_page_opts = group_db::GroupFilterOptions {
        page: u32::MAX,
        ..group_db::GroupFilterOptions::default()
    };
    let groups = group_db::get_groups(&db, &user, last_page_opts)
        .await
        .expect("a huge page must saturate, not overflow");
    assert!(groups.items.is_empty());
    assert_eq!(groups.page, u32::MAX);
}

#[tokio::test]
async fn get_models_file_type_filter_matches_every_stored_spelling() {
    let (_dir, db) = test_db().await;
    let user = User::default();

    // In-place imports keep the caller's spelling; the rest are zipped on import.
    for (sha, filetype) in [("a", "STL"), ("b", "stl.zip"), ("c", "stp"), ("d", "3mf")] {
        let blob_id = blob_db::add_blob(&db, sha, filetype, 1, None)
            .await
            .unwrap();
        model_db::add_model(&db, &user, sha, blob_id, None, None)
            .await
            .unwrap();
    }

    let names = |models: Vec<db::model::Model>| -> Vec<String> {
        let mut names: Vec<String> = models.into_iter().map(|m| m.name).collect();
        names.sort();
        names
    };

    let stl = model_db::get_models(
        &db,
        &user,
        model_db::ModelFilterOptions {
            file_types: Some(vec![FileType::Stl]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(names(stl.items), ["a", "b"]);

    let step_or_3mf = model_db::get_models(
        &db,
        &user,
        model_db::ModelFilterOptions {
            file_types: Some(vec![FileType::ZippedStep, FileType::Threemf]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(names(step_or_3mf.items), ["c", "d"]);

    let none = model_db::get_models(
        &db,
        &user,
        model_db::ModelFilterOptions {
            file_types: Some(vec![]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(none.items.is_empty());
}

// The share endpoint relies on this contract: it passes the share's ids as `Some(..)`,
// so `Some(vec![])` must match nothing, while `None` means "no restriction" and
// returns every model the user owns.
#[tokio::test]
async fn get_models_empty_model_ids_returns_nothing_while_none_returns_all() {
    let (_dir, db) = test_db().await;
    let user = User::default();

    let blob_id = blob_db::add_blob(&db, "one", "stl", 1, None).await.unwrap();
    for name in ["one", "two"] {
        model_db::add_model(&db, &user, name, blob_id, None, None)
            .await
            .unwrap();
    }

    let empty_ids = model_db::get_models(
        &db,
        &user,
        model_db::ModelFilterOptions {
            model_ids: Some(vec![]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(empty_ids.items.is_empty());

    let unrestricted = model_db::get_models(
        &db,
        &user,
        model_db::ModelFilterOptions {
            model_ids: None,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(unrestricted.items.len(), 2);
}

async fn set_childs_on_label(
    db: &db_context::DbContext,
    user: &User,
    parent_label_id: i64,
    child_label_ids: Vec<i64>,
) {
    label_db::set_childs_on_label(db, user, parent_label_id, &child_label_ids)
        .await
        .unwrap();
}

// Reads `labels_labels` directly so these assertions don't depend on `label_db::get_labels`.
async fn child_ids_of(db: &db_context::DbContext, parent_label_id: i64) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT child_label_id FROM labels_labels WHERE parent_label_id = ? ORDER BY child_label_id",
    )
    .bind(parent_label_id)
    .fetch_all(db)
    .await
    .unwrap()
}

async fn add_labels(db: &db_context::DbContext, user: &User, names: &[&str]) -> Vec<i64> {
    let mut ids = Vec::new();
    for name in names {
        ids.push(
            label_db::add_label(db, user, name, 0, None)
                .await
                .unwrap()
                .id,
        );
    }

    ids
}

#[tokio::test]
async fn set_childs_on_label_replaces_existing_children() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let ids = add_labels(&db, &user, &["parent", "a", "b", "c"]).await;
    let (parent, first_child, second_child, third_child) = (ids[0], ids[1], ids[2], ids[3]);
    set_childs_on_label(&db, &user, parent, vec![first_child, second_child]).await;

    set_childs_on_label(&db, &user, parent, vec![third_child]).await;

    assert_eq!(child_ids_of(&db, parent).await, vec![third_child]);
}

#[tokio::test]
async fn set_childs_on_label_with_empty_set_clears_children() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let ids = add_labels(&db, &user, &["parent", "a", "b"]).await;
    let (parent, first_child, second_child) = (ids[0], ids[1], ids[2]);
    set_childs_on_label(&db, &user, parent, vec![first_child, second_child]).await;

    set_childs_on_label(&db, &user, parent, vec![]).await;

    assert_eq!(child_ids_of(&db, parent).await, Vec::<i64>::new());
}

#[tokio::test]
async fn set_childs_on_label_with_overlapping_set_keeps_no_duplicates() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let ids = add_labels(&db, &user, &["parent", "a", "b", "c"]).await;
    let (parent, first_child, second_child, third_child) = (ids[0], ids[1], ids[2], ids[3]);
    set_childs_on_label(&db, &user, parent, vec![first_child, second_child]).await;

    set_childs_on_label(&db, &user, parent, vec![second_child, third_child]).await;

    assert_eq!(
        child_ids_of(&db, parent).await,
        vec![second_child, third_child]
    );
}

#[tokio::test]
async fn set_childs_on_label_leaves_other_labels_children_untouched() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let ids = add_labels(&db, &user, &["first", "second", "a", "b", "c"]).await;
    let (first_parent, second_parent, first_child, second_child, third_child) =
        (ids[0], ids[1], ids[2], ids[3], ids[4]);
    set_childs_on_label(&db, &user, first_parent, vec![first_child]).await;
    set_childs_on_label(&db, &user, second_parent, vec![second_child]).await;

    set_childs_on_label(&db, &user, first_parent, vec![third_child]).await;

    assert_eq!(child_ids_of(&db, first_parent).await, vec![third_child]);
    assert_eq!(child_ids_of(&db, second_parent).await, vec![second_child]);
}

#[tokio::test]
async fn get_labels_returns_labels_with_children_attached() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let ids = add_labels(&db, &user, &["parent", "child", "standalone"]).await;
    let (parent, child, standalone) = (ids[0], ids[1], ids[2]);
    set_childs_on_label(&db, &user, parent, vec![child]).await;

    let labels = label_db::get_labels(&db, &user, false).await.unwrap();

    let mut label_ids: Vec<i64> = labels.iter().map(|label| label.meta.id).collect();
    label_ids.sort_unstable();
    assert_eq!(label_ids, vec![parent, child, standalone]);
    let parent_label = labels.iter().find(|label| label.meta.id == parent).unwrap();
    assert_eq!(
        parent_label
            .children
            .iter()
            .map(|label| label.id)
            .collect::<Vec<_>>(),
        vec![child]
    );
    let child_label = labels.iter().find(|label| label.meta.id == child).unwrap();
    assert!(child_label.has_parent);
    assert!(child_label.children.is_empty());
}

#[tokio::test]
async fn get_models_reports_blob_added_not_model_added() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let blob_id = blob_db::add_blob(&db, "dated", "stl", 1, None)
        .await
        .unwrap();
    sqlx::query("UPDATE blobs SET blob_added = '2000-01-01T00:00:00Z' WHERE blob_id = ?")
        .bind(blob_id)
        .execute(&db)
        .await
        .unwrap();
    let model_id = model_db::add_model(&db, &user, "dated", blob_id, None, None)
        .await
        .unwrap();

    let models = model_db::get_models_via_ids(&db, &user, vec![model_id])
        .await
        .unwrap();

    assert_eq!(models[0].blob.added, "2000-01-01T00:00:00Z");
    assert_ne!(models[0].added, models[0].blob.added);
}

/// Inserts `count` models sharing one blob, name and timestamps, so every sort key ties, and
/// returns all of the user's model ids in ascending order.
async fn add_tied_models(db: &db_context::DbContext, user: &User, count: u32) -> Vec<i64> {
    let blob_id = blob_db::add_blob(db, "tied", "stl", 1, None).await.unwrap();
    sqlx::query(
        "WITH RECURSIVE sequence(number) AS (SELECT 1 UNION ALL SELECT number + 1 FROM sequence WHERE number < ?)
         INSERT INTO models (model_name, model_blob_id, model_added, model_user_id, model_unique_global_id, model_last_modified)
         SELECT 'tied', ?, '2000-01-01T00:00:00Z', ?, printf('%032x', number), '2000-01-01T00:00:00Z' FROM sequence",
    )
    .bind(count)
    .bind(blob_id)
    .bind(user.id)
    .execute(db)
    .await
    .unwrap();

    sqlx::query_scalar("SELECT model_id FROM models WHERE model_user_id = ? ORDER BY model_id")
        .bind(user.id)
        .fetch_all(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn get_all_models_returns_every_match_past_one_page_in_model_id_order_when_sort_keys_tie() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let model_ids = add_tied_models(&db, &user, db::MAX_PAGE_SIZE + 1).await;

    // The paging fields are ignored, so they must not shrink the result.
    let models = model_db::get_all_models(
        &db,
        &user,
        model_db::ModelFilterOptions {
            order_by: Some(model_db::ModelOrderBy::NameAsc),
            page: 2,
            page_size: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let listed_ids: Vec<i64> = models.iter().map(|model| model.id).collect();
    assert_eq!(listed_ids, model_ids);
}

#[tokio::test]
async fn get_groups_lists_a_group_whose_models_sit_past_the_first_page() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    add_tied_models(&db, &user, db::MAX_PAGE_SIZE).await;
    let blob_id = blob_db::add_blob(&db, "late", "stl", 1, None)
        .await
        .unwrap();
    let late_model_id = model_db::add_model(&db, &user, "late", blob_id, None, None)
        .await
        .unwrap();
    let group_id = group_db::add_empty_group(&db, &user, "late", None)
        .await
        .unwrap()
        .id;
    group_db::set_group_id_on_models(&db, &user, Some(group_id), vec![late_model_id], None)
        .await
        .unwrap();

    let groups = group_db::get_groups(&db, &user, group_db::GroupFilterOptions::default())
        .await
        .unwrap();

    assert_eq!(groups.items.len(), 1);
    assert_eq!(groups.items[0].meta.id, group_id);
    assert_eq!(
        groups.items[0]
            .models
            .iter()
            .map(|model| model.id)
            .collect::<Vec<_>>(),
        vec![late_model_id]
    );
}

#[tokio::test]
async fn get_models_label_filter_keeps_every_label_of_a_matching_model() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let label_ids = add_labels(&db, &user, &["wanted", "other"]).await;
    let (wanted_label_id, other_label_id) = (label_ids[0], label_ids[1]);
    let blob_id = blob_db::add_blob(&db, "labeled", "stl", 1, None)
        .await
        .unwrap();
    let carrier_id = model_db::add_model(&db, &user, "carrier", blob_id, None, None)
        .await
        .unwrap();
    let other_only_id = model_db::add_model(&db, &user, "other-only", blob_id, None, None)
        .await
        .unwrap();
    label_db::add_labels_on_models(&db, &user, &label_ids, &[carrier_id], None)
        .await
        .unwrap();
    label_db::add_labels_on_models(&db, &user, &[other_label_id], &[other_only_id], None)
        .await
        .unwrap();

    let models = model_db::get_models(
        &db,
        &user,
        model_db::ModelFilterOptions {
            label_ids: Some(vec![wanted_label_id]),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .items;

    assert_eq!(
        models.iter().map(|model| model.id).collect::<Vec<_>>(),
        vec![carrier_id]
    );
    let mut carried_label_ids: Vec<i64> = models[0].labels.iter().map(|label| label.id).collect();
    carried_label_ids.sort_unstable();
    assert_eq!(carried_label_ids, vec![wanted_label_id, other_label_id]);
}
