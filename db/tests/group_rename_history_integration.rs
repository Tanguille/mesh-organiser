//! Local rename events preserve unknown ancestry and commit atomically with group edits.

use db::{
    group_db,
    group_history::{self, HistoryOrigin},
    model::user::User,
    user_db,
};

mod common;

async fn history(
    db: &db::db_context::DbContext,
    user: &User,
    id: &str,
) -> group_history::GroupHistory {
    let result = group_history::get_group_history(db, user, id)
        .await
        .unwrap();
    assert!(
        result.is_some(),
        "owned groups need a history with unknown ancestry"
    );
    result.unwrap()
}

#[tokio::test]
async fn local_renames_form_a_chain_but_pre_upgrade_ancestry_stays_unknown() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "original", None)
        .await
        .unwrap();
    let initial = history(&db, &user, &group.unique_global_id).await;
    assert!(matches!(initial.ancestry, HistoryOrigin::UnknownPreUpgrade));
    assert_eq!(initial.events, vec![]);
    assert_eq!(initial.frontier, vec![]);

    let timestamp = "2026-10-24T12:00:00Z";
    group_db::edit_group(&db, &user, group.id, "first", Some(timestamp), None)
        .await
        .unwrap();
    let first = history(&db, &user, &group.unique_global_id).await;
    assert!(matches!(first.ancestry, HistoryOrigin::UnknownPreUpgrade));
    assert_eq!(first.events.len(), 1);
    assert_eq!(first.events[0].name, "first");
    assert_eq!(first.events[0].group_global_id, group.unique_global_id);
    assert_eq!(first.events[0].parents, vec![]);
    assert_eq!(first.frontier, vec![first.events[0].id.clone()]);

    group_db::edit_group(&db, &user, group.id, "second", Some(timestamp), None)
        .await
        .unwrap();
    let second = history(&db, &user, &group.unique_global_id).await;
    assert!(matches!(second.ancestry, HistoryOrigin::UnknownPreUpgrade));
    assert_eq!(second.events.len(), 2);
    assert_eq!(second.events[1].name, "second");
    assert_eq!(second.events[1].parents, vec![first.events[0].id.clone()]);
    assert_eq!(second.frontier, vec![second.events[1].id.clone()]);
    assert_eq!(
        second.events[1].id.replica_id,
        first.events[0].id.replica_id
    );
    assert!(second.events[1].id.counter > first.events[0].id.counter);
    assert_ne!(second.events[0].id.replica_id, "");

    group_db::edit_group(&db, &user, group.id, "second", Some(timestamp), None)
        .await
        .unwrap();
    let no_op = history(&db, &user, &group.unique_global_id).await;
    assert_eq!(no_op.events.len(), 2);
    assert_eq!(no_op.frontier, second.frontier);
}

#[tokio::test]
async fn history_is_owner_scoped_and_survives_database_reopen() {
    let (dir, db) = common::test_db().await;
    let owner = User::default();
    let foreign_id =
        user_db::add_user(&db, "foreign", "foreign@example.test", &db::random_hex_32())
            .await
            .unwrap();
    let foreign = user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &owner, "before", None)
        .await
        .unwrap();
    group_db::edit_group(&db, &owner, group.id, "after", None, None)
        .await
        .unwrap();
    let before = history(&db, &owner, &group.unique_global_id).await;
    assert!(
        group_history::get_group_history(&db, &foreign, &group.unique_global_id)
            .await
            .unwrap()
            .is_none()
    );
    db.close().await;
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    let after = history(&db, &owner, &group.unique_global_id).await;
    assert!(matches!(after.ancestry, HistoryOrigin::UnknownPreUpgrade));
    assert_eq!(after.frontier, before.frontier);
    assert_eq!(after.events.len(), 1);
    assert_eq!(after.events[0].id, before.events[0].id);
    assert_eq!(after.events[0].name, "after");
}

#[tokio::test]
async fn failed_event_insert_rolls_back_the_group_rename() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "before", None)
        .await
        .unwrap();
    let before = history(&db, &user, &group.unique_global_id).await;
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
    sqlx::query("CREATE TRIGGER reject_history BEFORE INSERT ON group_causal_events BEGIN SELECT RAISE(ABORT, 'injected history failure'); END").execute(&db).await.unwrap();
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
    assert_eq!(name, "before");
    assert_eq!(after_revision, revision);
    let after_head: i64 = sqlx::query_scalar(
        "SELECT head FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(user.id)
    .bind(&group.unique_global_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(after_head, head);
    let after = history(&db, &user, &group.unique_global_id).await;
    assert_eq!(after.frontier, before.frontier);
    assert_eq!(after.events.len(), before.events.len());
    sqlx::query("DROP TRIGGER reject_history")
        .execute(&db)
        .await
        .unwrap();
    group_db::edit_group(&db, &user, group.id, "after", None, None)
        .await
        .unwrap();
    assert_eq!(
        history(&db, &user, &group.unique_global_id)
            .await
            .events
            .len(),
        1
    );
}

#[tokio::test]
async fn upgrade_preserves_unknown_ancestry_for_existing_groups() {
    let dir = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(dir.path().join("db.sqlite"))
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let migrations = sqlx::migrate!("./migrations")
        .iter()
        .filter(|migration| migration.version < 20_261_112_190_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(migrations)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let group = common::legacy_group(&db, &user, "legacy").await;
    db.close().await;
    let backup = dir.path().join("backups");
    std::fs::create_dir_all(&backup).unwrap();
    let db = db::db_context::setup_db(&dir.path().join("db.sqlite"), &backup).await;
    let initial = history(&db, &user, &group.unique_global_id).await;
    assert!(matches!(initial.ancestry, HistoryOrigin::UnknownPreUpgrade));
    assert_eq!(initial.events, vec![]);
    assert_eq!(initial.frontier, vec![]);
    group_db::edit_group(&db, &user, group.id, "after upgrade", None, None)
        .await
        .unwrap();
    let after = history(&db, &user, &group.unique_global_id).await;
    assert!(matches!(after.ancestry, HistoryOrigin::UnknownPreUpgrade));
    assert_eq!(after.events.len(), 1);
    assert_eq!(after.events[0].parents, vec![]);
}

#[tokio::test]
async fn unrelated_groups_have_independent_frontiers_and_ambiguous_ids_fail_closed() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let first = group_db::add_empty_group(&db, &user, "first", None)
        .await
        .unwrap();
    let second = group_db::add_empty_group(&db, &user, "second", None)
        .await
        .unwrap();
    group_db::edit_group(&db, &user, first.id, "first renamed", None, None)
        .await
        .unwrap();
    group_db::edit_group(&db, &user, second.id, "second renamed", None, None)
        .await
        .unwrap();
    let first_history = history(&db, &user, &first.unique_global_id).await;
    let second_history = history(&db, &user, &second.unique_global_id).await;
    assert_eq!(first_history.events.len(), 1);
    assert_eq!(second_history.events.len(), 1);
    assert_eq!(first_history.events[0].name, "first renamed");
    assert_eq!(second_history.events[0].name, "second renamed");
    assert_eq!(first_history.events[0].parents, vec![]);
    assert_eq!(second_history.events[0].parents, vec![]);
    let duplicate =
        sqlx::query("UPDATE models_group SET group_unique_global_id = ? WHERE group_id = ?")
            .bind(&first.unique_global_id)
            .bind(second.id)
            .execute(&db)
            .await;
    if duplicate.is_ok() {
        assert!(
            group_history::get_group_history(&db, &user, &first.unique_global_id)
                .await
                .is_err()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_successful_renames_never_fork_the_local_frontier() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "original", None)
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        group_db::edit_group(&db, &user, group.id, "first", None, None),
        group_db::edit_group(&db, &user, group.id, "second", None, None),
    );
    let successes = usize::from(a.is_ok()) + usize::from(b.is_ok());
    assert!(successes > 0);
    let history = history(&db, &user, &group.unique_global_id).await;
    assert_eq!(history.events.len(), successes);
    assert_eq!(history.frontier.len(), 1);
    if successes == 2 {
        assert_eq!(
            history.events[1].parents,
            vec![history.events[0].id.clone()]
        );
    }
    assert_eq!(history.frontier[0], history.events.last().unwrap().id);
}
