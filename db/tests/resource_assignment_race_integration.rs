//! Project assignment must remain owner-scoped and atomic across races.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use sqlx::sqlite::SqliteConnectOptions;
use tokio::sync::Notify;

use db::{
    blob_db, db_context::DbContext, group_db, model::user::User, random_hex_32, resource_db,
    user_db,
};

mod common;

async fn populated_group(db: &DbContext, user: &User) -> i64 {
    let group = group_db::add_empty_group(db, user, "group", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(db, &random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let model = db::model_db::add_model(db, user, "model", blob, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(db, user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    group.id
}

async fn pre_allocator_database() -> (tempfile::TempDir, DbContext) {
    let dir = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(dir.path().join("db.sqlite"))
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    // Reproduce legacy project id reuse while executing the current assignment path.
    sqlx::query("DROP TRIGGER resource_monotonic_id")
        .execute(&db)
        .await
        .unwrap();
    (dir, db)
}

async fn populated_legacy_group(db: &DbContext, user: &User) -> i64 {
    let group = common::legacy_group(db, user, "group").await;
    let blob = blob_db::add_blob(db, &random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let model = sqlx::query("INSERT INTO models (model_name, model_blob_id, model_added, model_user_id, model_unique_global_id, model_last_modified) VALUES ('model', ?, '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')")
        .bind(blob).bind(user.id).bind(random_hex_32()).execute(db).await.unwrap().last_insert_rowid();
    group_db::set_group_id_on_models(db, user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    group.id
}

// Pause after the owned group and project lookups release the connection. A transactional
// implementation may finish before this point, in which case it serialized before the rival.
async fn controlled_pool(path: &std::path::Path) -> (DbContext, Arc<Notify>, Arc<Notify>) {
    let reached = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .after_release({
            let reached = reached.clone();
            let resume = resume.clone();
            let armed = armed.clone();
            let releases = releases.clone();
            move |_connection, _| {
                let reached = reached.clone();
                let resume = resume.clone();
                let armed = armed.clone();
                let releases = releases.clone();
                Box::pin(async move {
                    if armed.load(Ordering::SeqCst) && releases.fetch_add(1, Ordering::SeqCst) == 2
                    {
                        reached.notify_one();
                        resume.notified().await;
                    }
                    Ok(true)
                })
            }
        })
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .foreign_keys(true)
                .busy_timeout(Duration::from_secs(15)),
        )
        .await
        .unwrap();
    let connection = pool.acquire().await.unwrap();
    armed.store(true, Ordering::SeqCst);
    drop(connection);
    (pool, reached, resume)
}

#[tokio::test(flavor = "multi_thread")]
async fn deleted_project_reused_by_foreign_owner_cannot_be_attached() {
    let (dir, db) = pre_allocator_database().await;
    let user = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let group = populated_legacy_group(&db, &user).await;
    let resource_id = sqlx::query("INSERT INTO resources (resource_name, resource_created, resource_user_id, resource_unique_global_id, resource_last_modified) VALUES ('owned', '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')")
        .bind(user.id).bind(random_hex_32()).execute(&db).await.unwrap().last_insert_rowid();
    let (pool, reached, resume) = controlled_pool(&dir.path().join("db.sqlite")).await;
    let worker_user = user.clone();
    let mut task = tokio::spawn(async move {
        resource_db::set_resource_on_group(&pool, &worker_user, Some(resource_id), group, None)
            .await
    });
    tokio::select! {
        () = reached.notified() => {
            resource_db::delete_resource(&db, &user, resource_id).await.unwrap();
            sqlx::query("INSERT INTO resources (resource_id, resource_name, resource_created, resource_user_id, resource_unique_global_id, resource_last_modified) VALUES (?, 'foreign', '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')")
                .bind(resource_id).bind(other.id).bind(random_hex_32()).execute(&db).await.unwrap();
            resume.notify_one();
            assert!(task.await.unwrap().is_err(), "late assignment must reject reused foreign project");
        }
        result = &mut task => {
            result.unwrap().unwrap();
            resource_db::delete_resource(&db, &user, resource_id).await.unwrap();
            sqlx::query("INSERT INTO resources (resource_id, resource_name, resource_created, resource_user_id, resource_unique_global_id, resource_last_modified) VALUES (?, 'foreign', '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')")
                .bind(resource_id).bind(other.id).bind(random_hex_32()).execute(&db).await.unwrap();
        }
        () = tokio::time::sleep(Duration::from_secs(10)) => { resume.notify_one(); panic!("assignment did not reach a controlled outcome"); }
    }
    let attached: Option<i64> =
        sqlx::query_scalar("SELECT group_resource_id FROM models_group WHERE group_id = ?")
            .bind(group)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(attached, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn group_deletion_and_project_assignment_serialize_safely() {
    let (dir, db) = common::test_db().await;
    let user = User::default();
    let group = populated_group(&db, &user).await;
    let resource = resource_db::add_resource(&db, &user, "project", None)
        .await
        .unwrap();
    let (pool, reached, resume) = controlled_pool(&dir.path().join("db.sqlite")).await;
    let worker_user = user.clone();
    let mut task = tokio::spawn(async move {
        resource_db::set_resource_on_group(&pool, &worker_user, Some(resource.id), group, None)
            .await
    });
    tokio::select! {
        () = reached.notified() => {
            group_db::delete_group(&db, &user, group).await.unwrap();
            resume.notify_one();
            assert!(task.await.unwrap().is_err(), "deleted group must reject late project assignment");
            let attached: Option<i64> = sqlx::query_scalar("SELECT group_resource_id FROM models_group WHERE group_id = ?").bind(group).fetch_one(&db).await.unwrap();
            assert_eq!(attached, None);
        }
        result = &mut task => {
            result.unwrap().unwrap();
            group_db::delete_group(&db, &user, group).await.unwrap();
        }
        () = tokio::time::sleep(Duration::from_secs(10)) => { resume.notify_one(); panic!("assignment did not reach a controlled outcome"); }
    }
    let deleted: Option<String> =
        sqlx::query_scalar("SELECT group_deleted_at FROM models_group WHERE group_id = ?")
            .bind(group)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(deleted.is_some());
}

#[tokio::test]
async fn timestamp_failure_rolls_back_project_and_group_revision() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = populated_group(&db, &user).await;
    let resource = resource_db::add_resource(&db, &user, "project", None)
        .await
        .unwrap();
    let before: (Option<i64>, i64) = sqlx::query_as(
        "SELECT group_resource_id, group_revision FROM models_group WHERE group_id = ?",
    )
    .bind(group)
    .fetch_one(&db)
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER reject_project_timestamp BEFORE UPDATE OF resource_last_modified ON resources BEGIN SELECT RAISE(ABORT, 'timestamp failure'); END").execute(&db).await.unwrap();
    assert!(
        resource_db::set_resource_on_group(&db, &user, Some(resource.id), group, None)
            .await
            .is_err()
    );
    let after: (Option<i64>, i64) = sqlx::query_as(
        "SELECT group_resource_id, group_revision FROM models_group WHERE group_id = ?",
    )
    .bind(group)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn trashed_group_cannot_receive_a_project() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = populated_group(&db, &user).await;
    let resource = resource_db::add_resource(&db, &user, "project", None)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group).await.unwrap();
    assert!(
        resource_db::set_resource_on_group(&db, &user, Some(resource.id), group, None)
            .await
            .is_err()
    );
    let attached: Option<i64> =
        sqlx::query_scalar("SELECT group_resource_id FROM models_group WHERE group_id = ?")
            .bind(group)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(attached, None);
}
