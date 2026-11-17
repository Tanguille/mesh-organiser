//! Writable database initialization rotates the causal writer once per open.
use std::path::Path;

use futures::FutureExt;

use db::{
    causal_envelope::{self, EntityKind},
    db_context::DbContext,
    group_db, group_history,
    model::user::User,
    user_db,
};

mod common;

async fn writer(db: &DbContext) -> (String, i64) {
    sqlx::query_as("SELECT replica_id, counter FROM group_causal_replica WHERE singleton = 1")
        .fetch_one(db)
        .await
        .unwrap()
}
async fn open(path: &Path, backups: &Path) -> DbContext {
    db::db_context::setup_db(path, backups).await
}

#[tokio::test]
async fn reopen_rotates_once_and_preserves_birth_account_and_event_history() {
    let (dir, db) = common::test_db().await;
    let user = User::default();
    let initial = writer(&db).await;
    assert_eq!(initial.1, 0);
    let group = group_db::add_empty_group(&db, &user, "original", None)
        .await
        .unwrap();
    let birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    group_db::edit_group(&db, &user, group.id, "first", None, None)
        .await
        .unwrap();
    let first_history = group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    let initial_creation =
        causal_envelope::get_local_creation_record(&db, &user, EntityKind::Group, &birth)
            .await
            .unwrap()
            .unwrap();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let before = writer(&db).await;
    assert!(before.1 > 0);
    db.close().await;
    let db = open(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    let reopened = writer(&db).await;
    assert_ne!(
        reopened.0, before.0,
        "a writable database open must rotate its writer"
    );
    assert_eq!(reopened.1, 0);
    let again = group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.events, first_history.events);
    assert_eq!(again.frontier, first_history.frontier);
    assert_eq!(
        user_db::get_sync_identity(&db, user.id)
            .await
            .unwrap()
            .unwrap(),
        identity
    );
    let after_creation =
        causal_envelope::get_local_creation_record(&db, &user, EntityKind::Group, &birth)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(after_creation.envelope, initial_creation.envelope);
    let birth_after: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(birth_after, birth);
    let connection_a = db.acquire().await.unwrap();
    let connection_b = db.acquire().await.unwrap();
    drop(connection_a);
    drop(connection_b);
    assert_eq!(
        writer(&db).await,
        reopened,
        "pool connections must not rotate writers"
    );
    group_db::edit_group(&db, &user, group.id, "second", None, None)
        .await
        .unwrap();
    let after = group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.events.last().unwrap().id.replica_id, reopened.0);
    assert_eq!(after.events.last().unwrap().parents, first_history.frontier);
}

#[tokio::test]
async fn independently_opened_consistent_database_copies_use_distinct_writers() {
    let (dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "copied", None)
        .await
        .unwrap();
    group_db::edit_group(&db, &user, group.id, "before copy", None, None)
        .await
        .unwrap();
    let history = group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let copy_path = dir.path().join("copy.sqlite");
    sqlx::query("VACUUM INTO ?")
        .bind(copy_path.to_str().unwrap())
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    let original = open(
        &dir.path().join("db.sqlite"),
        &dir.path().join("original-backups"),
    )
    .await;
    let copied = open(&copy_path, &dir.path().join("copy-backups")).await;
    let original_writer = writer(&original).await;
    let copied_writer = writer(&copied).await;
    assert_ne!(
        original_writer.0, copied_writer.0,
        "separate copied databases need distinct writers"
    );
    assert_eq!((original_writer.1, copied_writer.1), (0, 0));
    for pool in [&original, &copied] {
        assert_eq!(
            user_db::get_sync_identity(pool, user.id)
                .await
                .unwrap()
                .unwrap(),
            identity
        );
        let current = group_history::get_group_history(pool, &user, &group.unique_global_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.events, history.events);
        assert_eq!(current.frontier, history.frontier);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_file_pools_allocate_unique_chained_dots_after_second_open_rotates() {
    let (dir, first) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&first, &user, "start", None)
        .await
        .unwrap();
    group_db::edit_group(&first, &user, group.id, "before", None, None)
        .await
        .unwrap();
    let previous = group_history::get_group_history(&first, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    let writer_before = writer(&first).await;
    let second = open(
        &dir.path().join("db.sqlite"),
        &dir.path().join("second-backups"),
    )
    .await;
    let after_open = writer(&second).await;
    assert_ne!(writer_before.0, after_open.0);
    assert_eq!(after_open.1, 0);
    let mut tasks = Vec::new();
    for index in 0..12 {
        let pool = if index % 2 == 0 {
            first.clone()
        } else {
            second.clone()
        };
        let user = user.clone();
        let id = group.id;
        tasks.push(tokio::spawn(async move {
            group_db::edit_group(&pool, &user, id, &format!("name-{index}"), None, None)
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let after = group_history::get_group_history(&first, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.events.len(), previous.events.len() + 12);
    let dots: std::collections::HashSet<_> = after
        .events
        .iter()
        .map(|e| (e.id.replica_id.clone(), e.id.counter))
        .collect();
    assert_eq!(dots.len(), after.events.len());
    for pair in after.events.windows(2) {
        assert_eq!(pair[1].parents, vec![pair[0].id.clone()]);
    }
    assert_eq!(writer(&first).await, writer(&second).await);
}

#[tokio::test]
async fn failed_rotation_aborts_initialization_and_preserves_writer_and_history() {
    let (dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "before", None)
        .await
        .unwrap();
    let before = writer(&db).await;
    let history = group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    sqlx::query("CREATE TRIGGER reject_writer_rotation BEFORE UPDATE ON group_causal_replica BEGIN SELECT RAISE(ABORT, 'injected failure'); END").execute(&db).await.unwrap();
    db.close().await;
    let attempted = std::panic::AssertUnwindSafe(open(
        &dir.path().join("db.sqlite"),
        &dir.path().join("failed-backups"),
    ))
    .catch_unwind()
    .await;
    assert!(
        attempted.is_err(),
        "initialization must fail closed when writer rotation fails"
    );
    let raw = sqlx::SqlitePool::connect(&format!(
        "sqlite:{}",
        dir.path().join("db.sqlite").display()
    ))
    .await
    .unwrap();
    assert_eq!(writer(&raw).await, before);
    let after = group_history::get_group_history(&raw, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.events, history.events);
    assert_eq!(after.frontier, history.frontier);
    sqlx::query("DROP TRIGGER reject_writer_rotation")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    let reopened = open(
        &dir.path().join("db.sqlite"),
        &dir.path().join("recovered-backups"),
    )
    .await;
    let recovered = writer(&reopened).await;
    assert_ne!(recovered.0, before.0);
    assert_eq!(recovered.1, 0);
}
