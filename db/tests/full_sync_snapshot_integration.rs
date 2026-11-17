//! A full snapshot binds every owner-scoped component to one database read view.
mod common;
use db::{
    blob_db, group_db, model::user::User, model_db, random_hex_32, resource_db, sync_snapshot,
    user_db,
};

async fn model(db: &db::db_context::DbContext, user: &User, name: &str) -> (i64, String) {
    let blob = blob_db::add_blob(db, name, "stl", 1, None).await.unwrap();
    let id = model_db::add_model(db, user, name, blob, None, None)
        .await
        .unwrap();
    let global = sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap();
    (id, global)
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn full_snapshot_binds_owner_identity_and_all_owned_state() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let identity = user_db::get_sync_identity(&db, owner.id)
        .await
        .unwrap()
        .unwrap();
    let other_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &owner, "owner group", None)
        .await
        .unwrap();
    let foreign = group_db::add_empty_group(&db, &other, "foreign", None)
        .await
        .unwrap();
    let removed = group_db::add_empty_group(&db, &owner, "deleted", None)
        .await
        .unwrap();
    let project = resource_db::add_resource(&db, &owner, "project", None)
        .await
        .unwrap();
    let foreign_project = resource_db::add_resource(&db, &other, "foreign", None)
        .await
        .unwrap();
    let (live, live_global) = model(&db, &owner, "owner-live").await;
    let (trash, trash_global) = model(&db, &owner, "owner-trash").await;
    let (_, foreign_global) = model(&db, &other, "foreign-model").await;
    group_db::set_group_id_on_models(&db, &owner, Some(group.id), vec![live, trash], None)
        .await
        .unwrap();
    resource_db::set_resource_on_group(&db, &owner, Some(project.id), group.id, None)
        .await
        .unwrap();
    model_db::trash_models(&db, &owner, &[trash]).await.unwrap();
    group_db::delete_group(&db, &owner, removed.id)
        .await
        .unwrap();
    let result = sync_snapshot::get_full_sync_snapshot(&db, &owner, &identity.account_id).await;
    assert!(result.is_ok(), "an owned consistent snapshot must succeed");
    let full = result.unwrap();
    assert_eq!(full.identity.instance_id, identity.instance_id);
    assert_eq!(full.identity.account_id, identity.account_id);
    assert_eq!(full.groups.len(), 2);
    let active = full
        .groups
        .iter()
        .find(|g| g.global_id == group.unique_global_id)
        .unwrap();
    assert_eq!(active.name, "owner group");
    assert!(active.birth_id.is_some());
    assert_eq!(
        active.resource_global_id.as_deref(),
        Some(project.unique_global_id.as_str())
    );
    let deleted = full
        .groups
        .iter()
        .find(|g| g.global_id == removed.unique_global_id)
        .unwrap();
    assert!(deleted.deleted_at.is_some());
    assert!(
        full.deletions
            .iter()
            .any(|d| d.group_global_id == removed.unique_global_id)
    );
    assert!(
        full.heads
            .iter()
            .any(|h| h.group_global_id == removed.unique_global_id)
    );
    assert!(
        !full
            .groups
            .iter()
            .any(|g| g.global_id == foreign.unique_global_id)
    );
    assert_eq!(full.assignments.len(), 2);
    for (global, is_trashed) in [(&live_global, false), (&trash_global, true)] {
        let item = full
            .assignments
            .iter()
            .find(|m| &m.model_global_id == global)
            .unwrap();
        assert!(item.model_birth_id.is_some());
        assert_eq!(item.deleted_at.is_some(), is_trashed);
        assert_eq!(
            item.group_global_id.as_deref(),
            Some(group.unique_global_id.as_str())
        );
        assert_ne!(item.incarnation, "");
        let row: (i64, i64) = sqlx::query_as("SELECT model_assignment_revision, model_lifecycle_revision FROM models WHERE model_unique_global_id = ? AND model_user_id = ?").bind(global).bind(owner.id).fetch_one(&db).await.unwrap();
        assert_eq!((item.revision, item.lifecycle_revision), row);
    }
    assert!(
        !full
            .assignments
            .iter()
            .any(|m| m.model_global_id == foreign_global)
    );
    assert_eq!(full.resources.len(), 1);
    assert_eq!(full.resources[0].global_id, project.unique_global_id);
    assert!(
        !full
            .resources
            .iter()
            .any(|p| p.global_id == foreign_project.unique_global_id)
    );
}

#[tokio::test]
async fn account_replacement_and_missing_identities_fail_closed() {
    let (_dir, db) = common::test_db().await;
    let id = user_db::add_user(&db, "old", "old@example.test", &random_hex_32())
        .await
        .unwrap();
    let old = user_db::get_user_by_id(&db, id).await.unwrap().unwrap();
    let old_account = user_db::get_sync_identity(&db, id)
        .await
        .unwrap()
        .unwrap()
        .account_id;
    sqlx::query("DELETE FROM users WHERE user_id = ?")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (user_id, user_name, user_email, user_password_hash, user_created_at) VALUES (?, 'replacement', 'replacement@example.test', 'unused', '2020-01-01T00:00:00Z')").bind(id).execute(&db).await.unwrap();
    let new_account = user_db::get_sync_identity(&db, id)
        .await
        .unwrap()
        .unwrap()
        .account_id;
    assert_ne!(old_account, new_account);
    assert!(
        sync_snapshot::get_full_sync_snapshot(&db, &old, &old_account)
            .await
            .is_err()
    );
    assert!(
        sync_snapshot::get_full_sync_snapshot(&db, &old, &new_account)
            .await
            .is_ok()
    );
    assert!(
        sync_snapshot::get_full_sync_snapshot(
            &db,
            &User {
                id: -1,
                ..User::default()
            },
            &new_account
        )
        .await
        .is_err()
    );
    sqlx::query("DELETE FROM sync_instance")
        .execute(&db)
        .await
        .unwrap();
    assert!(
        sync_snapshot::get_full_sync_snapshot(&db, &old, &new_account)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn duplicate_identities_and_cross_owner_relationships_fail_closed() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let identity = user_db::get_sync_identity(&db, owner.id)
        .await
        .unwrap()
        .unwrap();
    let foreign_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &owner, "owner", None)
        .await
        .unwrap();
    let project = resource_db::add_resource(&db, &other, "foreign", None)
        .await
        .unwrap();
    sqlx::query("UPDATE models_group SET group_resource_id = ? WHERE group_id = ?")
        .bind(project.id)
        .bind(group.id)
        .execute(&db)
        .await
        .unwrap();
    assert!(
        sync_snapshot::get_full_sync_snapshot(&db, &owner, &identity.account_id)
            .await
            .is_err()
    );
    sqlx::query("UPDATE models_group SET group_resource_id = NULL WHERE group_id = ?")
        .bind(group.id)
        .execute(&db)
        .await
        .unwrap();
    let (_, one) = model(&db, &owner, "one").await;
    let (two, _) = model(&db, &owner, "two").await;
    sqlx::query("UPDATE models SET model_unique_global_id = ? WHERE model_id = ?")
        .bind(one)
        .bind(two)
        .execute(&db)
        .await
        .unwrap();
    assert!(
        sync_snapshot::get_full_sync_snapshot(&db, &owner, &identity.account_id)
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)]
async fn concurrent_group_assignment_and_project_changes_do_not_mix_read_views() {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::sync::Notify;
    let (dir, db) = common::test_db().await;
    let user = User::default();
    let account = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap()
        .account_id;
    let group = group_db::add_empty_group(&db, &user, "off", None)
        .await
        .unwrap();
    let project = resource_db::add_resource(&db, &user, "project", None)
        .await
        .unwrap();
    let (model_id, model_global) = model(&db, &user, "racing-model").await;
    let old_project_global = project.unique_global_id.clone();
    let new_project_global = random_hex_32();
    let reached = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let armed = Arc::new(AtomicBool::new(false));
    let releases = Arc::new(AtomicUsize::new(0));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .after_release({
            let reached = Arc::clone(&reached);
            let resume = Arc::clone(&resume);
            let armed = Arc::clone(&armed);
            let releases = Arc::clone(&releases);
            move |_connection, _| {
                let reached = Arc::clone(&reached);
                let resume = Arc::clone(&resume);
                let armed = Arc::clone(&armed);
                let releases = Arc::clone(&releases);
                Box::pin(async move {
                    if armed.load(Ordering::SeqCst) && releases.fetch_add(1, Ordering::SeqCst) == 1
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
                .filename(dir.path().join("db.sqlite"))
                .foreign_keys(true)
                .busy_timeout(Duration::from_secs(15)),
        )
        .await
        .unwrap();
    let connection = pool.acquire().await.unwrap();
    armed.store(true, Ordering::SeqCst);
    drop(connection);
    let reader_pool = pool.clone();
    let reader_user = user.clone();
    let mut reading = tokio::spawn(async move {
        sync_snapshot::get_full_sync_snapshot(&reader_pool, &reader_user, &account).await
    });
    let full = tokio::select! {
        () = reached.notified() => {
            let mut tx = db.begin().await.unwrap();
            sqlx::query("UPDATE resources SET resource_unique_global_id = ? WHERE resource_id = ?").bind(&new_project_global).bind(project.id).execute(&mut *tx).await.unwrap();
            sqlx::query("UPDATE models_group SET group_name = 'on', group_resource_id = ? WHERE group_id = ?").bind(project.id).bind(group.id).execute(&mut *tx).await.unwrap();
            sqlx::query("UPDATE models SET model_group_id = ? WHERE model_id = ?").bind(group.id).bind(model_id).execute(&mut *tx).await.unwrap();
            tx.commit().await.unwrap();
            resume.notify_one();
            reading.await.unwrap().unwrap()
        }
        result = &mut reading => result.unwrap().unwrap(),
        () = tokio::time::sleep(Duration::from_secs(10)) => { resume.notify_one(); panic!("snapshot did not reach a controlled outcome"); }
    };
    let folder = full
        .groups
        .iter()
        .find(|g| g.global_id == group.unique_global_id)
        .unwrap();
    let assignment = full
        .assignments
        .iter()
        .find(|m| m.model_global_id == model_global)
        .unwrap();
    let resource = full.resources.first().unwrap();
    match folder.name.as_str() {
        "off" => {
            assert_eq!(folder.resource_global_id, None);
            assert_eq!(assignment.group_global_id, None);
            assert_eq!(resource.global_id, old_project_global);
        }
        "on" => {
            assert_eq!(
                folder.resource_global_id.as_deref(),
                Some(new_project_global.as_str())
            );
            assert_eq!(
                assignment.group_global_id.as_deref(),
                Some(group.unique_global_id.as_str())
            );
            assert_eq!(resource.global_id, new_project_global);
        }
        other => panic!("unexpected group state {other}"),
    }
}

#[tokio::test]
async fn snapshot_preserves_explicitly_unknown_legacy_births() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let account = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap()
        .account_id;
    let legacy_group = common::legacy_group(&db, &user, "legacy").await;
    let legacy_model = common::legacy_model(&db, &user, "legacy-model").await;
    let global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(legacy_model)
            .fetch_one(&db)
            .await
            .unwrap();
    let full = sync_snapshot::get_full_sync_snapshot(&db, &user, &account)
        .await
        .unwrap();
    assert_eq!(
        full.groups
            .iter()
            .find(|g| g.global_id == legacy_group.unique_global_id)
            .unwrap()
            .birth_id,
        None
    );
    assert_eq!(
        full.assignments
            .iter()
            .find(|m| m.model_global_id == global)
            .unwrap()
            .model_birth_id,
        None
    );
}
