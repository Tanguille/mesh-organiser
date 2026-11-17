//! Model lifecycle revisions prevent stale assignment snapshots from surviving Trash ABA.
mod common;

use db::{
    blob_db,
    db_context::DbContext,
    group_db::{self, AssignmentBatchResult, AssignmentChange},
    model::user::User,
    model_db, random_hex_32,
};

async fn model(db: &DbContext, user: &User) -> i64 {
    let blob = blob_db::add_blob(db, &random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    model_db::add_model(db, user, "model", blob, None, None)
        .await
        .unwrap()
}

async fn state(db: &DbContext, id: i64) -> (Option<String>, i64, i64) {
    sqlx::query_as("SELECT model_deleted, model_assignment_revision, model_lifecycle_revision FROM models WHERE model_id = ?")
        .bind(id).fetch_one(db).await.unwrap()
}

async fn change(db: &DbContext, user: &User, id: i64) -> AssignmentChange {
    let global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap();
    let snapshot = group_db::get_group_sync_snapshot(db, user).await.unwrap();
    let item = snapshot
        .assignments
        .iter()
        .find(|item| item.model_global_id == global)
        .unwrap();
    AssignmentChange {
        model_global_id: global,
        expected_incarnation: item.incarnation.clone(),
        expected_group_global_id: item.group_global_id.clone(),
        expected_revision: item.revision,
        expected_lifecycle_revision: item.lifecycle_revision,
        expected_deleted_at: item.deleted_at.clone(),
        target_group_global_id: item.group_global_id.clone(),
    }
}

#[tokio::test]
async fn trash_restore_aba_invalidates_stale_snapshot_and_exact_replay_is_stable() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let id = model(&db, &user).await;
    let stale = change(&db, &user, id).await;
    let before = state(&db, id).await;
    assert_eq!(before, (None, 0, 0));
    model_db::trash_models(&db, &user, &[id]).await.unwrap();
    assert_eq!(state(&db, id).await.2, 1);
    model_db::restore_models(&db, &user, &[id]).await.unwrap();
    assert_eq!(state(&db, id).await, (None, 0, 2));
    assert!(
        group_db::apply_assignment_batch(&db, &user, "stale", &[stale], &[])
            .await
            .is_err()
    );
    let current = change(&db, &user, id).await;
    assert_eq!(current.expected_lifecycle_revision, 2);
    assert_eq!(
        group_db::apply_assignment_batch(&db, &user, "fresh", std::slice::from_ref(&current), &[])
            .await
            .unwrap(),
        AssignmentBatchResult::Applied
    );
    assert_eq!(
        group_db::apply_assignment_batch(&db, &user, "fresh", std::slice::from_ref(&current), &[])
            .await
            .unwrap(),
        AssignmentBatchResult::AlreadyApplied
    );
    let mut changed = current;
    changed.expected_lifecycle_revision += 1;
    assert!(
        group_db::apply_assignment_batch(&db, &user, "fresh", &[changed], &[])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn no_op_lifecycle_updates_do_not_advance_revision() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let id = model(&db, &user).await;
    let other = model(&db, &user).await;
    model_db::restore_models(&db, &user, &[id]).await.unwrap();
    assert_eq!(state(&db, id).await.2, 0);
    model_db::trash_models(&db, &user, &[id, id]).await.unwrap();
    model_db::trash_models(&db, &user, &[id]).await.unwrap();
    sqlx::query("UPDATE models SET model_deleted = '2000-01-01T00:00:00Z' WHERE model_id = ?")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(state(&db, id).await.2, 1);
    model_db::untrash_models(&db, &user, &[id, id])
        .await
        .unwrap();
    model_db::untrash_models(&db, &user, &[id]).await.unwrap();
    assert_eq!(state(&db, id).await, (None, 0, 2));
    assert_eq!(state(&db, other).await, (None, 0, 0));
}

#[tokio::test]
async fn failed_restore_rolls_back_lifecycle_revision() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let id = model(&db, &user).await;
    model_db::trash_models(&db, &user, &[id]).await.unwrap();
    let before = state(&db, id).await;
    assert!(
        model_db::restore_models(&db, &user, &[id, i64::MAX])
            .await
            .is_err()
    );
    assert_eq!(state(&db, id).await, before);
}

#[tokio::test]
async fn exhausted_lifecycle_revision_rolls_back_trash() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let id = model(&db, &user).await;
    sqlx::query("UPDATE models SET model_lifecycle_revision = ? WHERE model_id = ?")
        .bind(i64::MAX)
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    assert!(model_db::trash_models(&db, &user, &[id]).await.is_err());
    assert_eq!(state(&db, id).await, (None, 0, i64::MAX));
}

#[tokio::test]
async fn legacy_journal_payload_cannot_replay_as_revision_zero() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let id = model(&db, &user).await;
    let current = change(&db, &user, id).await;
    assert_eq!(current.expected_lifecycle_revision, 0);
    let legacy = format!(
        "m{}:{}{}:{}-1:0--",
        current.model_global_id.len(),
        current.model_global_id,
        current.expected_incarnation.len(),
        current.expected_incarnation,
    );
    sqlx::query("INSERT INTO assignment_batch_journal (user_id, operation_id, payload) VALUES (?, 'legacy', ?)")
        .bind(user.id).bind(legacy).execute(&db).await.unwrap();
    assert!(
        group_db::apply_assignment_batch(&db, &user, "legacy", &[current], &[])
            .await
            .is_err()
    );
    assert_eq!(state(&db, id).await, (None, 0, 0));
}

#[tokio::test]
async fn migration_backfills_existing_live_and_trashed_models() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let migrations = sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version < 20_261_113_030_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(migrations)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let live = common::legacy_model(&db, &user, "live").await;
    let trashed = common::legacy_model(&db, &user, "trashed").await;
    sqlx::query("UPDATE models SET model_deleted = '2020-01-01T00:00:00Z' WHERE model_id = ?")
        .bind(trashed)
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    let upgraded = db::db_context::setup_db(&path, &dir.path().join("backups")).await;
    assert_eq!(state(&upgraded, live).await, (None, 0, 0));
    assert_eq!(state(&upgraded, trashed).await.2, 0);
    model_db::restore_models(&upgraded, &user, &[trashed])
        .await
        .unwrap();
    assert_eq!(state(&upgraded, trashed).await, (None, 0, 1));
}

#[tokio::test]
async fn foreign_owner_cannot_change_lifecycle_or_submit_assignment() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let foreign_id =
        db::user_db::add_user(&db, "foreign", "foreign@example.test", &random_hex_32())
            .await
            .unwrap();
    let foreign = db::user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let id = model(&db, &owner).await;
    let request = change(&db, &owner, id).await;
    model_db::trash_models(&db, &foreign, &[id]).await.unwrap();
    assert_eq!(state(&db, id).await, (None, 0, 0));
    assert!(
        model_db::restore_models(&db, &foreign, &[id])
            .await
            .is_err()
    );
    assert!(
        group_db::apply_assignment_batch(&db, &foreign, "foreign", &[request], &[])
            .await
            .is_err()
    );
    assert_eq!(state(&db, id).await, (None, 0, 0));
}

#[tokio::test]
async fn stale_model_rejects_entire_batch_without_journal_entry() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let first = model(&db, &user).await;
    let second = model(&db, &user).await;
    let fresh = change(&db, &user, first).await;
    let stale = change(&db, &user, second).await;
    model_db::trash_models(&db, &user, &[second]).await.unwrap();
    model_db::restore_models(&db, &user, &[second])
        .await
        .unwrap();
    assert!(
        group_db::apply_assignment_batch(&db, &user, "mixed", &[fresh, stale], &[])
            .await
            .is_err()
    );
    let entries: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assignment_batch_journal WHERE user_id = ? AND operation_id = 'mixed'")
        .bind(user.id).fetch_one(&db).await.unwrap();
    assert_eq!(entries, 0);
    assert_eq!(state(&db, first).await, (None, 0, 0));
    assert_eq!(state(&db, second).await, (None, 0, 2));
}

#[tokio::test]
async fn legacy_empty_batch_journal_cannot_replay() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    sqlx::query("INSERT INTO assignment_batch_journal (user_id, operation_id, payload) VALUES (?, 'empty-legacy', '')")
        .bind(user.id).execute(&db).await.unwrap();
    assert!(
        group_db::apply_assignment_batch(&db, &user, "empty-legacy", &[], &[])
            .await
            .is_err()
    );
}
