//! Real Trash operations record their causal effects with the exact deletion event.
use db::{
    group_db,
    group_history::{self, GroupLifecycleAction, HistoryOrigin, MembershipAction},
    model::user::User,
    model_db, random_hex_32, user_db,
};

mod common;

async fn model(db: &db::db_context::DbContext, user: &User, name: &str) -> (i64, String, String) {
    let blob = db::blob_db::add_blob(db, name, "stl", 1, None)
        .await
        .unwrap();
    let id = model_db::add_model(db, user, name, blob, None, None)
        .await
        .unwrap();
    let (global, incarnation) = sqlx::query_as(
        "SELECT model_unique_global_id, model_incarnation FROM models WHERE model_id = ?",
    )
    .bind(id)
    .fetch_one(db)
    .await
    .unwrap();
    (id, global, incarnation)
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn local_delete_and_restore_record_precisely_tagged_live_and_trashed_members() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let other_id = user_db::add_user(&db, "foreign", "foreign@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let destination = group_db::add_empty_group(&db, &user, "destination", None)
        .await
        .unwrap();
    let (live, live_global, live_inc) = model(&db, &user, "live").await;
    let (trashed, trash_global, trash_inc) = model(&db, &user, "trashed").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![live, trashed], None)
        .await
        .unwrap();
    group_db::edit_group(&db, &user, group.id, "renamed", None, None)
        .await
        .unwrap();
    let rename = group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap()
        .frontier;
    model_db::trash_models(&db, &user, &[trashed])
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let request = common::group_restore_request(&db, &user, group.id).await;
    let lifecycle = group_history::get_group_lifecycle_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap();
    assert!(
        lifecycle.is_some(),
        "an owned deletion must have lifecycle history"
    );
    let lifecycle = lifecycle.unwrap();
    assert_eq!(lifecycle.len(), 1);
    let deletion = &lifecycle[0];
    assert_eq!(deletion.parents, rename);
    assert_eq!(
        deletion.action,
        GroupLifecycleAction::Delete {
            deletion_operation_id: request.deletion_operation_id.clone()
        }
    );
    let delete_dot = deletion.id.clone();
    for (global, incarnation) in [(&live_global, &live_inc), (&trash_global, &trash_inc)] {
        let history = group_history::get_membership_history(&db, &user, global, incarnation)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(history.ancestry, HistoryOrigin::UnknownPreUpgrade));
        assert_eq!(history.events.len(), 2);
        assert_eq!(
            history.events[0].action,
            MembershipAction::Move {
                from_group_global_id: None,
                to_group_global_id: Some(group.unique_global_id.clone()),
            }
        );
        assert_eq!(
            history.events[1].parents,
            vec![history.events[0].id.clone()]
        );
        assert_eq!(
            history.events[1].action,
            MembershipAction::Detach {
                group_global_id: group.unique_global_id.clone(),
                deletion_operation_id: request.deletion_operation_id.clone(),
                deletion_event: delete_dot.clone()
            }
        );
        assert_eq!(history.frontier, vec![history.events[1].id.clone()]);
        assert!(
            group_history::get_membership_history(&db, &other, global, incarnation)
                .await
                .unwrap()
                .is_none()
        );
    }
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    assert_eq!(
        group_history::get_group_lifecycle_history(&db, &user, &group.unique_global_id)
            .await
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    group_db::set_group_id_on_models(&db, &user, Some(destination.id), vec![live], None)
        .await
        .unwrap();
    group_db::restore_groups(&db, &user, std::slice::from_ref(&request), 30)
        .await
        .unwrap();
    let lifecycle = group_history::get_group_lifecycle_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lifecycle.len(), 2);
    assert_eq!(lifecycle[1].parents, vec![delete_dot.clone()]);
    assert_eq!(
        lifecycle[1].action,
        GroupLifecycleAction::Restore {
            deletion_operation_id: request.deletion_operation_id.clone(),
            deletion_event: Some(delete_dot.clone())
        }
    );
    let after_group = group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_group.frontier, vec![lifecycle[1].id.clone()]);
    assert!(matches!(
        after_group.ancestry,
        HistoryOrigin::UnknownPreUpgrade
    ));
    let live_history = group_history::get_membership_history(&db, &user, &live_global, &live_inc)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        live_history.events.len(),
        3,
        "moved members must not receive a false restoration event"
    );
    assert_eq!(
        live_history.events[2].parents,
        vec![live_history.events[1].id.clone()]
    );
    assert_eq!(
        live_history.events[2].action,
        MembershipAction::Move {
            from_group_global_id: None,
            to_group_global_id: Some(destination.unique_global_id.clone()),
        }
    );
    let trash_history =
        group_history::get_membership_history(&db, &user, &trash_global, &trash_inc)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(trash_history.events.len(), 3);
    assert_eq!(
        trash_history.events[2].parents,
        vec![trash_history.events[1].id.clone()]
    );
    assert_eq!(
        trash_history.events[2].action,
        MembershipAction::Restore {
            group_global_id: group.unique_global_id.clone(),
            deletion_operation_id: request.deletion_operation_id.clone(),
            deletion_event: Some(delete_dot)
        }
    );
    group_db::restore_groups(&db, &user, &[request], 30)
        .await
        .unwrap();
    assert_eq!(
        group_history::get_group_lifecycle_history(&db, &user, &group.unique_global_id)
            .await
            .unwrap()
            .unwrap()
            .len(),
        2
    );
    sqlx::query("UPDATE models SET model_deleted = '2000-01-01T00:00:00Z' WHERE model_id = ?")
        .bind(trashed)
        .execute(&db)
        .await
        .unwrap();
    model_db::purge_trashed_models(&db, 30).await.unwrap();
    let retained = group_history::get_membership_history(&db, &user, &trash_global, &trash_inc)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        retained.events.len(),
        3,
        "model purge must not erase membership history"
    );
}

#[tokio::test]
async fn legacy_trash_restore_records_unknown_delete_event_without_inventing_ancestry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|item| item.version < 20_261_112_190_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let group = common::legacy_group(&db, &user, "legacy").await;
    let deletion_id = random_hex_32();
    sqlx::query("UPDATE models_group SET group_deleted_at = '2099-01-01T00:00:00Z', group_deletion_operation_id = ? WHERE group_id = ?").bind(&deletion_id).bind(group.id).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO group_deletion_records (user_id, group_global_id, operation_id, deleted_at, group_revision) VALUES (?, ?, ?, '2099-01-01T00:00:00Z', 0)").bind(user.id).bind(&group.unique_global_id).bind(&deletion_id).execute(&db).await.unwrap();
    db.close().await;
    let db = db::db_context::setup_db(&path, &dir.path().join("backups")).await;
    let request = common::group_restore_request(&db, &user, group.id).await;
    group_db::restore_groups(&db, &user, &[request], 30)
        .await
        .unwrap();
    let group_history = group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        group_history.ancestry,
        HistoryOrigin::UnknownPreUpgrade
    ));
    let events = group_history::get_group_lifecycle_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap();
    assert!(events.is_some());
    let events = events.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].action,
        GroupLifecycleAction::Restore {
            deletion_operation_id: deletion_id,
            deletion_event: None
        }
    );
    assert_eq!(events[0].parents, []);
}

#[tokio::test]
async fn failed_second_detach_rolls_back_all_domain_and_history_changes() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let (one, _, _) = model(&db, &user, "one").await;
    let (two, _, _) = model(&db, &user, "two").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![one, two], None)
        .await
        .unwrap();
    let counter: i64 =
        sqlx::query_scalar("SELECT counter FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    let before: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM group_causal_events), (SELECT COUNT(*) FROM membership_causal_events), (SELECT COUNT(*) FROM group_recovery_members), (SELECT COUNT(*) FROM group_deletion_records)",
    ).fetch_one(&db).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_second_detach BEFORE INSERT ON causal_events WHEN NEW.encoding = 'local_history_v1' AND json_extract(NEW.envelope, '$.action.kind') = 'detach' AND (SELECT COUNT(*) FROM membership_causal_event_facts WHERE event_kind = 'detach') = 1 BEGIN SELECT RAISE(ABORT, 'injected second detach failure'); END").execute(&db).await.unwrap();
    let error = group_db::delete_group(&db, &user, group.id)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("injected second detach failure"));
    for id in [one, two] {
        let assigned: Option<i64> =
            sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(assigned, Some(group.id));
    }
    let trashed: Option<String> =
        sqlx::query_scalar("SELECT group_deleted_at FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(trashed, None);
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM group_causal_events), (SELECT COUNT(*) FROM membership_causal_events), (SELECT COUNT(*) FROM group_recovery_members), (SELECT COUNT(*) FROM group_deletion_records)",
    ).fetch_one(&db).await.unwrap();
    assert_eq!(counts, before, "history and recovery must roll back");
    let after: i64 =
        sqlx::query_scalar("SELECT counter FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(after, counter);
}

#[tokio::test]
async fn failed_restore_history_insert_leaves_group_and_models_in_trash() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let (id, global, incarnation) = model(&db, &user, "part").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![id], None)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let request = common::group_restore_request(&db, &user, group.id).await;
    let counter: i64 =
        sqlx::query_scalar("SELECT counter FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    let before = group_history::get_membership_history(&db, &user, &global, &incarnation)
        .await
        .unwrap()
        .unwrap();
    sqlx::query("CREATE TRIGGER reject_restore_history BEFORE INSERT ON group_causal_events BEGIN SELECT RAISE(ABORT, 'injected history failure'); END").execute(&db).await.unwrap();
    assert!(
        group_db::restore_groups(&db, &user, &[request], 30)
            .await
            .is_err()
    );
    assert_eq!(
        common::group_restore_request(&db, &user, group.id).await.id,
        group.id
    );
    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(assigned, None);
    let after = group_history::get_membership_history(&db, &user, &global, &incarnation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.events, before.events);
    assert_eq!(after.frontier, before.frontier);
    let after_counter: i64 =
        sqlx::query_scalar("SELECT counter FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(after_counter, counter);
}
