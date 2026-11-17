//! Group Trash cleanup follows configured retention while keeping deletion records for offline sync.

mod common;

use db::{blob_db, group_db, model::user::User, model_db};
use service::{AppState, export_service};

async fn seed_deleted_group(
    app_state: &AppState,
    name: &str,
    deleted_at: &str,
) -> (i64, i64, String) {
    let user = User::default();
    let group = group_db::add_empty_group(&app_state.db, &user, name, None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&app_state.db, name, "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&app_state.db, &user, name, blob, None, None)
        .await
        .unwrap();
    sqlx::query("UPDATE models_group SET group_deleted_at = ? WHERE group_id = ?")
        .bind(deleted_at)
        .bind(group.id)
        .execute(&*app_state.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO group_recovery_members(group_id, model_id) VALUES (?, ?)")
        .bind(group.id)
        .bind(model)
        .execute(&*app_state.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO group_deletion_records(user_id, group_global_id, operation_id, deleted_at) VALUES (?, ?, ?, ?)")
        .bind(user.id)
        .bind(&group.unique_global_id)
        .bind(format!("{name}-operation"))
        .bind(deleted_at)
        .execute(&*app_state.db)
        .await
        .unwrap();

    (group.id, model, group.unique_global_id)
}

async fn group_exists(app_state: &AppState, group_id: i64) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models_group WHERE group_id = ?)")
        .bind(group_id)
        .fetch_one(&*app_state.db)
        .await
        .unwrap()
}

async fn assert_expired_group_cleanup(startup: bool) {
    let (_dir, app_state) = common::test_app_state().await;
    app_state.configuration.lock().unwrap().trash_retention_days = 30;
    let (expired_id, model_id, global_id) =
        seed_deleted_group(&app_state, "expired-group", "2000-01-01T00:00:00Z").await;
    let (recent_id, _, _) = seed_deleted_group(&app_state, "recent-group", &db::time_now()).await;
    let active = group_db::add_empty_group(&app_state.db, &User::default(), "active", None)
        .await
        .unwrap();

    if startup {
        export_service::cleanup_library_on_startup(&app_state)
            .await
            .unwrap();
    } else {
        export_service::purge_expired_trash(&app_state)
            .await
            .unwrap();
    }

    assert!(
        !group_exists(&app_state, expired_id).await,
        "expired group metadata must be purged"
    );
    assert!(
        group_exists(&app_state, recent_id).await,
        "recent group remains recoverable"
    );
    assert!(
        group_exists(&app_state, active.id).await,
        "active empty groups remain intact"
    );
    let model_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models WHERE model_id = ?)")
            .bind(model_id)
            .fetch_one(&*app_state.db)
            .await
            .unwrap();
    assert!(
        model_exists,
        "purging a group must not delete former models"
    );
    let snapshot_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM group_recovery_members WHERE group_id = ?")
            .bind(expired_id)
            .fetch_one(&*app_state.db)
            .await
            .unwrap();
    assert_eq!(snapshot_count, 0);
    let retained_records: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM group_deletion_records WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(User::default().id)
    .bind(global_id)
    .fetch_one(&*app_state.db)
    .await
    .unwrap();
    assert_eq!(
        retained_records, 1,
        "an offline device still needs the deletion record"
    );
}

#[tokio::test]
async fn startup_purges_expired_group_recovery_but_keeps_its_deletion_record() {
    assert_expired_group_cleanup(true).await;
}

#[tokio::test]
async fn regular_purge_removes_expired_group_recovery_but_keeps_its_deletion_record() {
    assert_expired_group_cleanup(false).await;
}

#[tokio::test]
async fn configured_retention_is_applied_to_group_recovery() {
    let (_dir, app_state) = common::test_app_state().await;
    app_state.configuration.lock().unwrap().trash_retention_days = 36_500;
    let (id, _, _) = seed_deleted_group(&app_state, "long-retention", "2000-01-01T00:00:00Z").await;

    export_service::purge_expired_trash(&app_state)
        .await
        .unwrap();
    assert!(group_exists(&app_state, id).await);
    app_state.configuration.lock().unwrap().trash_retention_days = 30;
    export_service::purge_expired_trash(&app_state)
        .await
        .unwrap();
    assert!(!group_exists(&app_state, id).await);
}
