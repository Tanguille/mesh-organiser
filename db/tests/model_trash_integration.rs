//! Pins the trash bin (soft delete): trashed models vanish from every normal read and count,
//! keep their group, labels and blob until purged, and trash/restore only touch the caller's
//! own rows.

use db::{
    blob_db,
    db_context::{self, DbContext},
    group_db, label_db,
    model::{ModelFlags, user::User},
    model_db, random_hex_32, time_now, user_db,
};

const SET_DELETED_SQL: &str = "UPDATE models SET model_deleted = ? WHERE model_id = ?";
const SET_LAST_MODIFIED_SQL: &str = "UPDATE models SET model_last_modified = ? WHERE model_id = ?";

async fn test_db() -> (tempfile::TempDir, DbContext) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("db.sqlite");
    let backup_dir = dir.path().join("backups");
    std::fs::create_dir_all(&backup_dir).unwrap();
    let db = db_context::setup_db(&db_path, &backup_dir).await;

    (dir, db)
}

async fn add_other_user(db: &DbContext) -> User {
    let user_id = user_db::add_user(db, "other", "other@example.com", &random_hex_32())
        .await
        .unwrap();

    user_db::get_user_by_id(db, user_id).await.unwrap().unwrap()
}

/// Adds a model backed by its own blob; `sha256` doubles as the model name.
async fn add_model(db: &DbContext, user: &User, sha256: &str) -> i64 {
    let blob_id = blob_db::add_blob(db, sha256, "stl", 1, None).await.unwrap();

    model_db::add_model(db, user, sha256, blob_id, None, None)
        .await
        .unwrap()
}

async fn add_group(db: &DbContext, user: &User, name: &str, model_ids: Vec<i64>) -> i64 {
    let group_id = group_db::add_empty_group(db, user, name, None)
        .await
        .unwrap()
        .id;
    group_db::set_group_id_on_models(db, user, Some(group_id), model_ids, None)
        .await
        .unwrap();

    group_id
}

async fn trash(db: &DbContext, user: &User, model_ids: &[i64]) {
    model_db::trash_models(db, user, model_ids).await.unwrap();
}

async fn set_model_column(db: &DbContext, sql: &'static str, value: &str, model_id: i64) {
    sqlx::query(sql)
        .bind(value)
        .bind(model_id)
        .execute(db)
        .await
        .unwrap();
}

// Raw column reads, so assertions don't depend on the row mapper under test.
async fn model_deleted(db: &DbContext, model_id: i64) -> Option<String> {
    sqlx::query_scalar("SELECT model_deleted FROM models WHERE model_id = ?")
        .bind(model_id)
        .fetch_one(db)
        .await
        .unwrap()
}

async fn model_last_modified(db: &DbContext, model_id: i64) -> String {
    sqlx::query_scalar("SELECT model_last_modified FROM models WHERE model_id = ?")
        .bind(model_id)
        .fetch_one(db)
        .await
        .unwrap()
}

async fn all_model_ids(db: &DbContext) -> Vec<i64> {
    sqlx::query_scalar("SELECT model_id FROM models ORDER BY model_id")
        .fetch_all(db)
        .await
        .unwrap()
}

async fn listed_model_ids(db: &DbContext, user: &User, trashed: bool) -> Vec<i64> {
    let mut ids: Vec<i64> = model_db::get_models(
        db,
        user,
        model_db::ModelFilterOptions {
            trashed,
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .items
    .into_iter()
    .map(|model| model.id)
    .collect();
    ids.sort_unstable();

    ids
}

async fn group_last_modified(db: &DbContext, group_id: i64) -> String {
    sqlx::query_scalar("SELECT group_last_modified FROM models_group WHERE group_id = ?")
        .bind(group_id)
        .fetch_one(db)
        .await
        .unwrap()
}

async fn label_last_modified(db: &DbContext, label_id: i64) -> String {
    sqlx::query_scalar("SELECT label_last_modified FROM labels WHERE label_id = ?")
        .bind(label_id)
        .fetch_one(db)
        .await
        .unwrap()
}

fn timestamp(year: u16) -> String {
    format!("{year}-01-01T00:00:00Z")
}

/// The ids of a model in its own group and with its own label.
struct GroupedLabeledModelIds {
    model: i64,
    group: i64,
    label: i64,
}

async fn add_grouped_labeled_model(
    db: &DbContext,
    user: &User,
    name: &str,
) -> GroupedLabeledModelIds {
    let model_id = add_model(db, user, name).await;
    let group_id = add_group(db, user, name, vec![model_id]).await;
    let label_id = label_db::add_label(db, user, name, 0, None)
        .await
        .unwrap()
        .id;
    label_db::add_labels_on_models(db, user, &[label_id], &[model_id], None)
        .await
        .unwrap();

    GroupedLabeledModelIds {
        model: model_id,
        group: group_id,
        label: label_id,
    }
}

/// Backdates the model, its group and its label to `last_modified`, so a later stamp shows.
async fn backdate(db: &DbContext, user: &User, ids: &GroupedLabeledModelIds, last_modified: &str) {
    set_model_column(db, SET_LAST_MODIFIED_SQL, last_modified, ids.model).await;
    group_db::set_last_updated_on_groups(db, user, &[ids.group], last_modified)
        .await
        .unwrap();
    label_db::set_last_updated_on_labels(db, user, &[ids.label], last_modified)
        .await
        .unwrap();
}

/// The last-modified of the model, its group and its label.
async fn last_modified_of(db: &DbContext, ids: &GroupedLabeledModelIds) -> [String; 3] {
    [
        model_last_modified(db, ids.model).await,
        group_last_modified(db, ids.group).await,
        label_last_modified(db, ids.label).await,
    ]
}

/// A `time_now()`-shaped timestamp `days` before now.
fn days_ago(days: i64) -> String {
    (chrono::Utc::now() - chrono::TimeDelta::days(days))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// `untrash_models` when `untrash` is set, else `restore_models`: the two share every rule but
/// the group and label stamp.
async fn restore_or_untrash(
    db: &DbContext,
    user: &User,
    ids: &[i64],
    untrash: bool,
) -> Result<(), db::DbError> {
    if untrash {
        model_db::untrash_models(db, user, ids).await
    } else {
        model_db::restore_models(db, user, ids).await
    }
}

fn assert_fresh_timestamp(value: &str, before: &str, after: &str) {
    assert!(
        value.len() == before.len() && before <= value && value <= after,
        "timestamp {value:?} is not a time_now() value between {before:?} and {after:?}"
    );
}

#[tokio::test]
async fn trash_models_hides_model_from_model_reads() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let live_id = add_model(&db, &user, "live").await;
    let trashed_id = add_model(&db, &user, "trashed").await;

    trash(&db, &user, &[trashed_id]).await;

    assert_eq!(listed_model_ids(&db, &user, false).await, vec![live_id]);
    let via_ids: Vec<i64> = model_db::get_models_via_ids(&db, &user, vec![live_id, trashed_id])
        .await
        .unwrap()
        .into_iter()
        .map(|model| model.id)
        .collect();
    assert_eq!(via_ids, vec![live_id]);
    assert!(
        model_db::get_model_via_id(&db, &user, trashed_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn trash_models_excludes_model_from_counts_and_size() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let live_blob_id = blob_db::add_blob(&db, "live-blob", "stl", 10, None)
        .await
        .unwrap();
    let trashed_blob_id = blob_db::add_blob(&db, "trashed-blob", "stl", 100, None)
        .await
        .unwrap();
    let live_favorite_id = model_db::add_model(&db, &user, "live", live_blob_id, None, None)
        .await
        .unwrap();
    let trashed_favorite_id =
        model_db::add_model(&db, &user, "trashed", trashed_blob_id, None, None)
            .await
            .unwrap();
    model_db::add_model(&db, &user, "plain", live_blob_id, None, None)
        .await
        .unwrap();
    for (model_id, name) in [(live_favorite_id, "live"), (trashed_favorite_id, "trashed")] {
        model_db::edit_model(
            &db,
            &user,
            model_id,
            name,
            None,
            None,
            ModelFlags::Favorite,
            None,
            None,
        )
        .await
        .unwrap();
    }

    trash(&db, &user, &[trashed_favorite_id]).await;

    assert_eq!(
        model_db::get_model_count(&db, &user, None).await.unwrap(),
        2
    );
    assert_eq!(
        model_db::get_model_count(&db, &user, Some(ModelFlags::Favorite))
            .await
            .unwrap(),
        1
    );
    let size = model_db::get_size_of_models(&db, &user).await.unwrap();
    assert_eq!(size.total_size, 20);
    assert!(
        size.blob_sha256.iter().all(|sha256| sha256 == "live-blob"),
        "trashed blob counted: {:?}",
        size.blob_sha256
    );
}

#[tokio::test]
async fn trash_models_excludes_model_from_label_model_count() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let live_grouped_id = add_model(&db, &user, "live-grouped").await;
    let trashed_grouped_id = add_model(&db, &user, "trashed-grouped").await;
    let live_ungrouped_id = add_model(&db, &user, "live-ungrouped").await;
    let trashed_ungrouped_id = add_model(&db, &user, "trashed-ungrouped").await;
    add_group(&db, &user, "live", vec![live_grouped_id]).await;
    add_group(&db, &user, "trashed", vec![trashed_grouped_id]).await;
    let label_id = label_db::add_label(&db, &user, "label", 0, None)
        .await
        .unwrap()
        .id;
    label_db::add_labels_on_models(
        &db,
        &user,
        &[label_id],
        &[
            live_grouped_id,
            trashed_grouped_id,
            live_ungrouped_id,
            trashed_ungrouped_id,
        ],
        None,
    )
    .await
    .unwrap();

    trash(&db, &user, &[trashed_grouped_id, trashed_ungrouped_id]).await;

    for (include_ungrouped_models, expected_groups) in [(false, 1), (true, 2)] {
        let labels = label_db::get_labels(&db, &user, include_ungrouped_models)
            .await
            .unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(
            (
                labels[0].model_count,
                labels[0].group_count,
                labels[0].self_group_count
            ),
            (2, expected_groups, expected_groups),
            "include_ungrouped_models: {include_ungrouped_models}"
        );
    }
}

#[tokio::test]
async fn trash_models_hides_fully_trashed_groups_and_their_counts() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let half_live_id = add_model(&db, &user, "half-live").await;
    let half_trashed_id = add_model(&db, &user, "half-trashed").await;
    let gone_id = add_model(&db, &user, "gone").await;
    let ungrouped_live_id = add_model(&db, &user, "ungrouped-live").await;
    let ungrouped_trashed_id = add_model(&db, &user, "ungrouped-trashed").await;
    let half_group_id = add_group(&db, &user, "half", vec![half_live_id, half_trashed_id]).await;
    add_group(&db, &user, "gone", vec![gone_id]).await;

    trash(
        &db,
        &user,
        &[half_trashed_id, gone_id, ungrouped_trashed_id],
    )
    .await;

    let groups = group_db::get_groups(&db, &user, group_db::GroupFilterOptions::default())
        .await
        .unwrap();
    assert_eq!(groups.items.len(), 1);
    assert_eq!(groups.items[0].meta.id, half_group_id);
    assert_eq!(
        groups.items[0]
            .models
            .iter()
            .map(|model| model.id)
            .collect::<Vec<_>>(),
        vec![half_live_id]
    );
    let with_ungrouped = group_db::get_groups(
        &db,
        &user,
        group_db::GroupFilterOptions {
            include_ungrouped_models: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut listed_model_ids: Vec<i64> = with_ungrouped
        .items
        .iter()
        .flat_map(|group| group.models.iter().map(|model| model.id))
        .collect();
    listed_model_ids.sort_unstable();
    assert_eq!(listed_model_ids, vec![half_live_id, ungrouped_live_id]);
    assert_eq!(
        group_db::get_group_count(&db, &user, false).await.unwrap(),
        1
    );
    assert_eq!(
        group_db::get_group_count(&db, &user, true).await.unwrap(),
        2
    );
}

#[tokio::test]
async fn trashed_filter_returns_only_trashed_models_with_deleted_set() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let live_id = add_model(&db, &user, "live").await;
    let trashed_id = add_model(&db, &user, "trashed").await;

    let before = time_now();
    trash(&db, &user, &[trashed_id]).await;
    let after = time_now();

    let trashed = model_db::get_models(
        &db,
        &user,
        model_db::ModelFilterOptions {
            trashed: true,
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .items;
    assert_eq!(trashed.len(), 1);
    assert_eq!(trashed[0].id, trashed_id);
    let deleted = trashed[0].deleted.as_deref().expect("deleted must be set");
    assert_fresh_timestamp(deleted, &before, &after);
    assert_eq!(
        model_deleted(&db, trashed_id).await.as_deref(),
        Some(deleted)
    );
    let live = model_db::get_model_via_id(&db, &user, live_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(live.deleted, None);
}

#[tokio::test]
async fn restore_models_clears_deleted_bumps_last_modified_and_keeps_labels_and_group() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let model_id = add_model(&db, &user, "model").await;
    let label_id = label_db::add_label(&db, &user, "label", 0, None)
        .await
        .unwrap()
        .id;
    label_db::add_labels_on_models(&db, &user, &[label_id], &[model_id], None)
        .await
        .unwrap();
    let group_id = add_group(&db, &user, "group", vec![model_id]).await;
    set_model_column(&db, SET_LAST_MODIFIED_SQL, &timestamp(2000), model_id).await;
    trash(&db, &user, &[model_id]).await;
    // Trashing must not bump last_modified: sync treats the missing model as deleted remotely.
    assert_eq!(model_last_modified(&db, model_id).await, timestamp(2000));

    let before = time_now();
    model_db::restore_models(&db, &user, &[model_id])
        .await
        .unwrap();
    let after = time_now();

    let model = model_db::get_model_via_id(&db, &user, model_id)
        .await
        .unwrap()
        .expect("restored model must be visible again");
    assert_eq!(model.deleted, None);
    assert_eq!(model_deleted(&db, model_id).await, None);
    assert_fresh_timestamp(&model.last_modified, &before, &after);
    assert_eq!(
        model
            .labels
            .iter()
            .map(|label| label.id)
            .collect::<Vec<_>>(),
        vec![label_id]
    );
    assert_eq!(model.group.map(|group| group.id), Some(group_id));
}

#[tokio::test]
async fn restore_models_stamps_the_restored_models_groups_and_labels_with_the_restore_time() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let restored = add_grouped_labeled_model(&db, &user, "restored").await;
    let still_trashed = add_grouped_labeled_model(&db, &user, "still-trashed").await;
    trash(&db, &user, &[restored.model, still_trashed.model]).await;
    backdate(&db, &user, &restored, &timestamp(2000)).await;
    backdate(&db, &user, &still_trashed, &timestamp(2000)).await;

    let before = time_now();
    model_db::restore_models(&db, &user, &[restored.model])
        .await
        .unwrap();
    let after = time_now();

    let [model_stamp, group_stamp, label_stamp] = last_modified_of(&db, &restored).await;
    assert_fresh_timestamp(&model_stamp, &before, &after);
    assert_eq!(group_stamp, model_stamp);
    assert_eq!(label_stamp, model_stamp);
    assert_eq!(
        last_modified_of(&db, &still_trashed).await,
        [timestamp(2000), timestamp(2000), timestamp(2000)]
    );
}

#[tokio::test]
async fn untrash_models_clears_deleted_and_stamps_only_the_model() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let untrashed = add_grouped_labeled_model(&db, &user, "untrashed").await;
    trash(&db, &user, &[untrashed.model]).await;
    backdate(&db, &user, &untrashed, &timestamp(2000)).await;

    let before = time_now();
    model_db::untrash_models(&db, &user, &[untrashed.model])
        .await
        .unwrap();
    let after = time_now();

    assert_eq!(model_deleted(&db, untrashed.model).await, None);
    let [model_stamp, group_stamp, label_stamp] = last_modified_of(&db, &untrashed).await;
    assert_fresh_timestamp(&model_stamp, &before, &after);
    assert_eq!(group_stamp, timestamp(2000));
    assert_eq!(label_stamp, timestamp(2000));
    let model = model_db::get_model_via_id(&db, &user, untrashed.model)
        .await
        .unwrap()
        .expect("untrashed model must be visible again");
    assert_eq!(model.group.map(|group| group.id), Some(untrashed.group));
    assert_eq!(
        model
            .labels
            .iter()
            .map(|label| label.id)
            .collect::<Vec<_>>(),
        vec![untrashed.label]
    );
}

#[tokio::test]
async fn restore_and_untrash_leave_live_models_and_their_group_and_labels_untouched() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let live = add_grouped_labeled_model(&db, &user, "live").await;
    backdate(&db, &user, &live, &timestamp(2000)).await;

    for untrash in [false, true] {
        restore_or_untrash(&db, &user, &[live.model], untrash)
            .await
            .unwrap();

        assert_eq!(model_deleted(&db, live.model).await, None);
        assert_eq!(
            last_modified_of(&db, &live).await,
            [timestamp(2000), timestamp(2000), timestamp(2000)],
            "untrash: {untrash}"
        );
    }
}

#[tokio::test]
async fn restore_and_untrash_with_an_unknown_or_foreign_id_fail_and_roll_back_the_whole_call() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let trashed = add_grouped_labeled_model(&db, &user, "trashed").await;
    let foreign_trashed_id = add_model(&db, &other_user, "foreign-trashed").await;
    trash(&db, &user, &[trashed.model]).await;
    trash(&db, &other_user, &[foreign_trashed_id]).await;
    set_model_column(&db, SET_DELETED_SQL, &timestamp(2001), trashed.model).await;
    backdate(&db, &user, &trashed, &timestamp(2000)).await;
    let unknown_id = foreign_trashed_id + 1000;

    for untrash in [false, true] {
        for refused_id in [unknown_id, foreign_trashed_id] {
            let result =
                restore_or_untrash(&db, &user, &[trashed.model, refused_id], untrash).await;

            assert!(
                matches!(result, Err(sqlx::Error::RowNotFound)),
                "untrash {untrash}, id {refused_id}: {result:?}"
            );
        }
    }

    assert_eq!(
        model_deleted(&db, trashed.model).await,
        Some(timestamp(2001))
    );
    assert_eq!(
        last_modified_of(&db, &trashed).await,
        [timestamp(2000), timestamp(2000), timestamp(2000)]
    );
    assert!(model_deleted(&db, foreign_trashed_id).await.is_some());
}

#[tokio::test]
async fn trash_and_restore_leave_another_users_rows_untouched() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let foreign_live_id = add_model(&db, &other_user, "foreign-live").await;
    let foreign_trashed_id = add_model(&db, &other_user, "foreign-trashed").await;
    trash(&db, &other_user, &[foreign_trashed_id]).await;
    set_model_column(&db, SET_DELETED_SQL, &timestamp(2001), foreign_trashed_id).await;
    set_model_column(
        &db,
        SET_LAST_MODIFIED_SQL,
        &timestamp(2000),
        foreign_trashed_id,
    )
    .await;

    trash(&db, &user, &[foreign_live_id]).await;
    let restore_result = model_db::restore_models(&db, &user, &[foreign_trashed_id]).await;

    assert!(
        matches!(restore_result, Err(sqlx::Error::RowNotFound)),
        "{restore_result:?}"
    );
    assert_eq!(model_deleted(&db, foreign_live_id).await, None);
    assert_eq!(
        model_deleted(&db, foreign_trashed_id).await,
        Some(timestamp(2001))
    );
    assert_eq!(
        model_last_modified(&db, foreign_trashed_id).await,
        timestamp(2000)
    );
    assert_eq!(
        listed_model_ids(&db, &other_user, false).await,
        vec![foreign_live_id]
    );
}

#[tokio::test]
async fn purge_trashed_models_deletes_only_rows_trashed_before_the_retention_across_users() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let own_expired_id = add_model(&db, &user, "own-expired").await;
    let foreign_expired_id = add_model(&db, &other_user, "foreign-expired").await;
    let within_retention_id = add_model(&db, &user, "within-retention").await;
    let recent_id = add_model(&db, &user, "recent").await;
    let live_id = add_model(&db, &user, "live").await;
    trash(
        &db,
        &user,
        &[own_expired_id, within_retention_id, recent_id],
    )
    .await;
    trash(&db, &other_user, &[foreign_expired_id]).await;
    for (model_id, deleted) in [
        (own_expired_id, days_ago(31)),
        (foreign_expired_id, timestamp(2000)),
        (within_retention_id, days_ago(29)),
    ] {
        set_model_column(&db, SET_DELETED_SQL, &deleted, model_id).await;
    }

    let purged = model_db::purge_trashed_models(&db, 30).await.unwrap();

    assert_eq!(purged, 2);
    assert_eq!(
        all_model_ids(&db).await,
        vec![within_retention_id, recent_id, live_id]
    );
}

#[tokio::test]
async fn dead_group_and_blob_cleanup_keep_those_of_trashed_models() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let blob_id = blob_db::add_blob(&db, "trashed-blob", "stl", 1, None)
        .await
        .unwrap();
    let model_id = model_db::add_model(&db, &user, "trashed", blob_id, None, None)
        .await
        .unwrap();
    let group_id = add_group(&db, &user, "group", vec![model_id]).await;
    trash(&db, &user, &[model_id]).await;

    group_db::delete_dead_groups(&db).await.unwrap();
    let dead_blobs = blob_db::get_and_delete_dead_blobs(&db).await.unwrap();

    assert!(dead_blobs.is_empty());
    assert_eq!(
        blob_db::get_blobs_via_ids(&db, vec![blob_id])
            .await
            .unwrap()
            .len(),
        1
    );
    model_db::restore_models(&db, &user, &[model_id])
        .await
        .unwrap();
    let model = model_db::get_model_via_id(&db, &user, model_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(model.group.map(|group| group.id), Some(group_id));
    assert_eq!(model.blob.id, blob_id);
}

#[tokio::test]
async fn sha256_lookups_still_find_trashed_models() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let model_id = add_model(&db, &user, "trashed-sha").await;
    trash(&db, &user, &[model_id]).await;

    let via_sha256 = model_db::get_model_id_via_sha256(&db, &user, "trashed-sha")
        .await
        .unwrap();
    let via_sha256s = model_db::get_model_ids_via_sha256s(&db, &user, &["trashed-sha".to_string()])
        .await
        .unwrap();

    assert_eq!(via_sha256, Some(model_id));
    assert_eq!(via_sha256s, vec![model_id]);
}

#[tokio::test]
async fn trash_and_restore_with_no_ids_change_nothing() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let live_id = add_model(&db, &user, "live").await;
    let trashed_id = add_model(&db, &user, "trashed").await;
    trash(&db, &user, &[trashed_id]).await;

    model_db::trash_models(&db, &user, &[]).await.unwrap();
    model_db::restore_models(&db, &user, &[]).await.unwrap();

    assert_eq!(listed_model_ids(&db, &user, false).await, vec![live_id]);
    assert_eq!(listed_model_ids(&db, &user, true).await, vec![trashed_id]);
}

#[tokio::test]
async fn trash_models_keeps_the_original_deleted_timestamp_of_a_trashed_model() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let model_id = add_model(&db, &user, "trashed").await;
    trash(&db, &user, &[model_id]).await;
    set_model_column(&db, SET_DELETED_SQL, &timestamp(2000), model_id).await;

    trash(&db, &user, &[model_id]).await;

    assert_eq!(model_deleted(&db, model_id).await, Some(timestamp(2000)));
}
