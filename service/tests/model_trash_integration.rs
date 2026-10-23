//! Pins the service side of the trash bin: delete moves models to the trash and leaves their
//! files alone, restore brings them back, and purging removes expired models with their files.

mod common;

use std::{
    fs,
    path::{Path, PathBuf},
};

use tempfile::TempDir;

use db::{blob_db, group_db, label_db, model::user::User, model_db, random_hex_32, user_db};
use service::{AppState, export_service, import_service::import_path, import_state::ImportState};

const SET_DELETED_SQL: &str = "UPDATE models SET model_deleted = ? WHERE model_id = ?";

struct StoredModel {
    id: i64,
    model_path: PathBuf,
    image_path: PathBuf,
}

/// Adds a model whose blob file and thumbnail exist on disk, as an import leaves them.
async fn add_model_with_files(app_state: &AppState, user: &User, sha256: &str) -> StoredModel {
    let blob_id = blob_db::add_blob(&app_state.db, sha256, "stl", 1, None)
        .await
        .unwrap();
    let blob = blob_db::get_blob_via_sha256(&app_state.db, sha256)
        .await
        .unwrap()
        .unwrap();
    let model_path = export_service::get_model_path_for_blob(&blob, app_state);
    let image_path = export_service::get_image_path_for_blob(&blob, app_state);
    fs::write(&model_path, sha256).unwrap();
    fs::write(&image_path, sha256).unwrap();
    let id = model_db::add_model(&app_state.db, user, sha256, blob_id, None, None)
        .await
        .unwrap();

    StoredModel {
        id,
        model_path,
        image_path,
    }
}

async fn add_other_user(app_state: &AppState) -> User {
    let user_id = user_db::add_user(
        &app_state.db,
        "other",
        "other@example.com",
        &random_hex_32(),
    )
    .await
    .unwrap();

    user_db::get_user_by_id(&app_state.db, user_id)
        .await
        .unwrap()
        .unwrap()
}

async fn set_model_deleted(app_state: &AppState, model_id: i64, deleted: &str) {
    sqlx::query(SET_DELETED_SQL)
        .bind(deleted)
        .bind(model_id)
        .execute(&*app_state.db)
        .await
        .unwrap();
}

/// In the order `get_trashed_models` returns them.
async fn trashed_model_ids(app_state: &AppState, user: &User) -> Vec<i64> {
    export_service::get_trashed_models(app_state, user)
        .await
        .unwrap()
        .into_iter()
        .map(|model| model.id)
        .collect()
}

async fn is_visible(app_state: &AppState, user: &User, model_id: i64) -> bool {
    model_db::get_model_via_id(&app_state.db, user, model_id)
        .await
        .unwrap()
        .is_some()
}

fn timestamp(year: u16) -> String {
    format!("{year}-01-01T00:00:00Z")
}

/// Imports the file at `path` for the default user and returns the ids of its models.
async fn import_file(app_state: &AppState, path: &Path) -> Vec<i64> {
    let import_state = import_path(
        path.to_str().unwrap(),
        app_state,
        ImportState::new(None, false, false, false, User::default()),
    )
    .await
    .unwrap();

    import_state.imported_models[0].model_ids.clone()
}

/// An STL file in its own temp dir, which must outlive the test.
fn gear_stl() -> (TempDir, PathBuf) {
    let root = TempDir::new().unwrap();
    let path = root.path().join("Gear.stl");
    fs::write(&path, b"solid gear\nendsolid gear\n").unwrap();

    (root, path)
}

async fn group_exists(app_state: &AppState, group_id: i64) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM models_group WHERE group_id = ?)")
        .bind(group_id)
        .fetch_one(&*app_state.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn delete_models_moves_model_to_trash_and_keeps_its_files() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let model = add_model_with_files(&app_state, &user, "trashed").await;

    export_service::delete_models(&app_state, &user, vec![model.id])
        .await
        .unwrap();

    assert!(!is_visible(&app_state, &user, model.id).await);
    assert_eq!(trashed_model_ids(&app_state, &user).await, vec![model.id]);
    assert!(
        model.model_path.exists(),
        "blob file must survive the trash"
    );
    assert!(
        model.image_path.exists(),
        "thumbnail must survive the trash"
    );
}

#[tokio::test]
async fn delete_models_rejects_a_model_the_user_does_not_own_and_trashes_nothing() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let other_user = add_other_user(&app_state).await;
    let own = add_model_with_files(&app_state, &user, "own").await;
    let foreign = add_model_with_files(&app_state, &other_user, "foreign").await;

    let result = export_service::delete_models(&app_state, &user, vec![own.id, foreign.id]).await;

    assert!(result.is_err());
    assert!(is_visible(&app_state, &user, own.id).await);
    assert!(is_visible(&app_state, &other_user, foreign.id).await);
}

#[tokio::test]
async fn restore_models_brings_a_trashed_model_back() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let model = add_model_with_files(&app_state, &user, "restored").await;
    export_service::delete_models(&app_state, &user, vec![model.id])
        .await
        .unwrap();

    export_service::restore_models(&app_state, &user, vec![model.id])
        .await
        .unwrap();

    assert_eq!(
        trashed_model_ids(&app_state, &user).await,
        Vec::<i64>::new()
    );
    let restored = model_db::get_model_via_id(&app_state.db, &user, model.id)
        .await
        .unwrap()
        .expect("restored model must be visible again");
    assert_eq!(restored.deleted, None);
}

#[tokio::test]
async fn restore_models_accepts_a_repeated_trashed_id() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let model = add_model_with_files(&app_state, &user, "repeated").await;
    export_service::delete_models(&app_state, &user, vec![model.id])
        .await
        .unwrap();

    export_service::restore_models(&app_state, &user, vec![model.id, model.id])
        .await
        .unwrap();

    assert_eq!(
        trashed_model_ids(&app_state, &user).await,
        Vec::<i64>::new()
    );
}

#[tokio::test]
async fn restore_models_refuses_a_live_unknown_or_foreign_id_and_restores_nothing() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let other_user = add_other_user(&app_state).await;
    let trashed = add_model_with_files(&app_state, &user, "trashed").await;
    let live = add_model_with_files(&app_state, &user, "live").await;
    let foreign = add_model_with_files(&app_state, &other_user, "foreign").await;
    export_service::delete_models(&app_state, &user, vec![trashed.id])
        .await
        .unwrap();
    export_service::delete_models(&app_state, &other_user, vec![foreign.id])
        .await
        .unwrap();
    let unknown_id = foreign.id + 1000;

    for refused_id in [live.id, unknown_id, foreign.id] {
        let result =
            export_service::restore_models(&app_state, &user, vec![trashed.id, refused_id]).await;

        assert!(
            matches!(
                &result,
                Err(service::ServiceError::InternalError(message))
                    if message == "Failed to find model to restore"
            ),
            "id {refused_id}: {result:?}"
        );
    }

    // The valid id sent alongside each refused one stays in the trash.
    assert_eq!(trashed_model_ids(&app_state, &user).await, vec![trashed.id]);
    assert_eq!(
        trashed_model_ids(&app_state, &other_user).await,
        vec![foreign.id]
    );
    assert!(is_visible(&app_state, &user, live.id).await);
}

#[tokio::test]
async fn get_trashed_models_lists_newest_deleted_first() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let oldest = add_model_with_files(&app_state, &user, "oldest").await;
    let newest = add_model_with_files(&app_state, &user, "newest").await;
    let middle = add_model_with_files(&app_state, &user, "middle").await;
    add_model_with_files(&app_state, &user, "live").await;
    export_service::delete_models(&app_state, &user, vec![oldest.id, newest.id, middle.id])
        .await
        .unwrap();
    for (model_id, year) in [(oldest.id, 2001), (newest.id, 2003), (middle.id, 2002)] {
        set_model_deleted(&app_state, model_id, &timestamp(year)).await;
    }

    let trashed = export_service::get_trashed_models(&app_state, &user)
        .await
        .unwrap();

    assert_eq!(
        trashed
            .iter()
            .map(|model| (model.id, model.deleted.clone()))
            .collect::<Vec<_>>(),
        vec![
            (newest.id, Some(timestamp(2003))),
            (middle.id, Some(timestamp(2002))),
            (oldest.id, Some(timestamp(2001))),
        ]
    );
}

#[tokio::test]
async fn get_trashed_models_returns_more_rows_than_a_page_holds() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let blob_id = blob_db::add_blob(&app_state.db, "shared", "stl", 1, None)
        .await
        .unwrap();
    let count = db::MAX_PAGE_SIZE + 1;
    // One statement: a thousand single inserts would dominate the run time.
    sqlx::query(
        "WITH RECURSIVE counter(number) AS (SELECT 1 UNION ALL SELECT number + 1 FROM counter WHERE number < ?)
         INSERT INTO models (model_name, model_blob_id, model_added, model_user_id, model_deleted)
         SELECT 'trashed-' || number, ?, '2024-01-01T00:00:00Z', ?, '2024-01-01T00:00:00Z' FROM counter",
    )
    .bind(count)
    .bind(blob_id)
    .bind(user.id)
    .execute(&*app_state.db)
    .await
    .unwrap();

    let trashed = export_service::get_trashed_models(&app_state, &user)
        .await
        .unwrap();

    assert_eq!(trashed.len(), count as usize);
}

#[tokio::test]
async fn purge_expired_trash_removes_expired_models_with_their_files_and_group() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let expired = add_model_with_files(&app_state, &user, "expired").await;
    let recent = add_model_with_files(&app_state, &user, "recent").await;
    let group_id = group_db::add_empty_group(&app_state.db, &user, "expired-group", None)
        .await
        .unwrap()
        .id;
    group_db::set_group_id_on_models(&app_state.db, &user, Some(group_id), vec![expired.id], None)
        .await
        .unwrap();
    export_service::delete_models(&app_state, &user, vec![expired.id, recent.id])
        .await
        .unwrap();
    set_model_deleted(&app_state, expired.id, &timestamp(2000)).await;

    export_service::purge_expired_trash(&app_state)
        .await
        .unwrap();

    assert_eq!(trashed_model_ids(&app_state, &user).await, vec![recent.id]);
    assert!(
        !expired.model_path.exists(),
        "expired blob file must be removed"
    );
    assert!(
        !expired.image_path.exists(),
        "expired thumbnail must be removed"
    );
    assert!(recent.model_path.exists());
    assert!(recent.image_path.exists());
    let remaining_groups: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM models_group WHERE group_id = ?")
            .bind(group_id)
            .fetch_one(&*app_state.db)
            .await
            .unwrap();
    assert_eq!(
        remaining_groups, 0,
        "the purged model's group must be removed"
    );
}

#[tokio::test]
async fn purge_expired_trash_sweeps_blobs_only_once_an_import_releases_the_lock() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let expired = add_model_with_files(&app_state, &user, "expired").await;
    export_service::delete_models(&app_state, &user, vec![expired.id])
        .await
        .unwrap();
    set_model_deleted(&app_state, expired.id, &timestamp(2000)).await;
    let import_guard = app_state.import_mutex.lock().await;

    let mut purge = std::pin::pin!(export_service::purge_expired_trash(&app_state));
    // A purge that skipped the lock would finish well within this on a test-sized database.
    let while_locked =
        tokio::time::timeout(std::time::Duration::from_millis(100), purge.as_mut()).await;

    assert!(while_locked.is_err(), "the purge finished under the lock");
    // Only the dead-blob sweep waits for the lock; the expired model row may already be gone.
    assert!(expired.model_path.exists(), "blob swept under the lock");
    assert!(
        blob_db::get_blob_via_sha256(&app_state.db, "expired")
            .await
            .unwrap()
            .is_some()
    );
    drop(import_guard);
    purge.await.unwrap();
    assert_eq!(
        trashed_model_ids(&app_state, &user).await,
        Vec::<i64>::new()
    );
    assert!(!expired.model_path.exists());
}

#[tokio::test]
async fn delete_user_skips_the_blob_sweep_while_an_import_holds_the_lock() {
    let (_dir, app_state) = common::test_app_state().await;
    let other_user = add_other_user(&app_state).await;
    let model = add_model_with_files(&app_state, &other_user, "owned").await;
    let import_guard = app_state.import_mutex.lock().await;

    // A thumbnail regeneration can hold the lock for hours, so the deletion must not wait on it.
    let while_locked = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        export_service::delete_user(&app_state, other_user.id),
    )
    .await;

    while_locked
        .expect("the user deletion waited for the lock")
        .unwrap();
    assert!(model.model_path.exists(), "blob swept under the lock");
    drop(import_guard);
    // A later sweep still finds the deleted user's blob dead.
    export_service::delete_dead_blobs(&app_state).await.unwrap();
    assert!(!model.model_path.exists());
}

#[tokio::test]
async fn purge_expired_trash_keeps_models_younger_than_the_configured_retention() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    // A century of retention, so a model trashed in 2000 is still within it.
    app_state.configuration.lock().unwrap().trash_retention_days = 36_500;
    let model = add_model_with_files(&app_state, &user, "kept").await;
    export_service::delete_models(&app_state, &user, vec![model.id])
        .await
        .unwrap();
    set_model_deleted(&app_state, model.id, &timestamp(2000)).await;

    export_service::purge_expired_trash(&app_state)
        .await
        .unwrap();

    assert_eq!(trashed_model_ids(&app_state, &user).await, vec![model.id]);
    assert!(model.model_path.exists());
    assert!(model.image_path.exists());
}

#[tokio::test]
async fn import_of_a_trashed_file_restores_the_same_model() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let (_root, path) = gear_stl();
    let model_id = import_file(&app_state, &path).await[0];
    export_service::delete_models(&app_state, &user, vec![model_id])
        .await
        .unwrap();

    let reimported = import_file(&app_state, &path).await;

    assert_eq!(reimported, vec![model_id]);
    assert_eq!(
        trashed_model_ids(&app_state, &user).await,
        Vec::<i64>::new()
    );
    let model = model_db::get_model_via_id(&app_state.db, &user, model_id)
        .await
        .unwrap()
        .expect("re-import must restore the trashed model");
    assert_eq!(model.deleted, None);
}

#[tokio::test]
async fn import_of_a_trashed_file_restores_it_without_stamping_its_group_or_labels() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let (_root, path) = gear_stl();
    let model_id = import_file(&app_state, &path).await[0];
    let group_id = group_db::add_empty_group(&app_state.db, &user, "gears", None)
        .await
        .unwrap()
        .id;
    group_db::set_group_id_on_models(&app_state.db, &user, Some(group_id), vec![model_id], None)
        .await
        .unwrap();
    let label_id = label_db::add_label(&app_state.db, &user, "gear", 0, None)
        .await
        .unwrap()
        .id;
    label_db::add_labels_on_models(&app_state.db, &user, &[label_id], &[model_id], None)
        .await
        .unwrap();
    group_db::set_last_updated_on_groups(&app_state.db, &user, &[group_id], &timestamp(2000))
        .await
        .unwrap();
    label_db::set_last_updated_on_labels(&app_state.db, &user, &[label_id], &timestamp(2000))
        .await
        .unwrap();
    export_service::delete_models(&app_state, &user, vec![model_id])
        .await
        .unwrap();

    let reimported = import_file(&app_state, &path).await;

    assert_eq!(reimported, vec![model_id]);
    let model = model_db::get_model_via_id(&app_state.db, &user, model_id)
        .await
        .unwrap()
        .expect("re-import must restore the trashed model");
    assert_eq!(model.group.map(|group| group.id), Some(group_id));
    assert_eq!(
        model
            .labels
            .iter()
            .map(|label| label.id)
            .collect::<Vec<_>>(),
        vec![label_id]
    );
    // Sync stamps the model right after; a stamp here would make this side's group and labels
    // look newest and revert the other side's edits.
    let group_last_modified: String =
        sqlx::query_scalar("SELECT group_last_modified FROM models_group WHERE group_id = ?")
            .bind(group_id)
            .fetch_one(&*app_state.db)
            .await
            .unwrap();
    let label_last_modified: String =
        sqlx::query_scalar("SELECT label_last_modified FROM labels WHERE label_id = ?")
            .bind(label_id)
            .fetch_one(&*app_state.db)
            .await
            .unwrap();
    assert_eq!(group_last_modified, timestamp(2000));
    assert_eq!(label_last_modified, timestamp(2000));
}

#[tokio::test]
async fn import_of_a_trashed_file_purged_before_the_restore_imports_it_afresh() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let (_root, path) = gear_stl();
    let model_id = import_file(&app_state, &path).await[0];
    let blob_id = model_db::get_model_via_id(&app_state.db, &user, model_id)
        .await
        .unwrap()
        .unwrap()
        .blob
        .id;
    export_service::delete_models(&app_state, &user, vec![model_id])
        .await
        .unwrap();
    // Stands in for a trash purge committing between the dedupe's sha256 lookup and its restore:
    // the row is gone by the time the restore checks it. The restore's rollback brings this row
    // back, which a real purge would not, so only the import's outcome is asserted.
    sqlx::query(
        "CREATE TRIGGER purge_on_untrash AFTER UPDATE OF model_deleted ON models
         WHEN OLD.model_deleted IS NOT NULL AND NEW.model_deleted IS NULL
         BEGIN DELETE FROM models WHERE model_id = NEW.model_id; END",
    )
    .execute(&*app_state.db)
    .await
    .unwrap();

    let reimported = import_file(&app_state, &path).await;

    assert_eq!(reimported.len(), 1);
    assert_ne!(reimported[0], model_id);
    let model = model_db::get_model_via_id(&app_state.db, &user, reimported[0])
        .await
        .unwrap()
        .expect("the fresh import must be a live model");
    assert_eq!(model.deleted, None);
    assert_eq!(
        model.blob.id, blob_id,
        "the fresh import reuses the blob row"
    );
}

#[tokio::test]
async fn purge_expired_trash_sweeps_dead_groups_and_blobs_only_when_it_purged_something() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let recent = add_model_with_files(&app_state, &user, "recent").await;
    export_service::delete_models(&app_state, &user, vec![recent.id])
        .await
        .unwrap();
    // Left empty for a moment by REST create-then-add, sync or the MCP server.
    let empty_group_id = group_db::add_empty_group(&app_state.db, &user, "empty", None)
        .await
        .unwrap()
        .id;
    blob_db::add_blob(&app_state.db, "orphan", "stl", 1, None)
        .await
        .unwrap();
    let orphan_blob = blob_db::get_blob_via_sha256(&app_state.db, "orphan")
        .await
        .unwrap()
        .unwrap();
    let orphan_path = export_service::get_model_path_for_blob(&orphan_blob, &app_state);
    fs::write(&orphan_path, "orphan").unwrap();

    export_service::purge_expired_trash(&app_state)
        .await
        .unwrap();

    assert_eq!(trashed_model_ids(&app_state, &user).await, vec![recent.id]);
    assert!(group_exists(&app_state, empty_group_id).await);
    assert!(
        blob_db::get_blob_via_sha256(&app_state.db, "orphan")
            .await
            .unwrap()
            .is_some()
    );
    assert!(orphan_path.exists());

    let expired = add_model_with_files(&app_state, &user, "expired").await;
    export_service::delete_models(&app_state, &user, vec![expired.id])
        .await
        .unwrap();
    set_model_deleted(&app_state, expired.id, &timestamp(2000)).await;

    export_service::purge_expired_trash(&app_state)
        .await
        .unwrap();

    assert!(!group_exists(&app_state, empty_group_id).await);
    assert!(
        blob_db::get_blob_via_sha256(&app_state.db, "orphan")
            .await
            .unwrap()
            .is_none()
    );
    assert!(!orphan_path.exists());
    assert!(recent.model_path.exists());
}
