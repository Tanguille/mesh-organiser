//! Ambiguous historical identities stay visible but cannot authorize a restore.
use db::{
    group_db::{self, GroupRestoreRequest},
    random_hex_32,
};

mod common;

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn ambiguous_legacy_groups_are_listed_but_cannot_be_restored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|item| item.version < 20_261_112_180_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let user = db::model::user::User::default();
    let first = common::legacy_group(&db, &user, "duplicate one").await;
    let second = common::legacy_group(&db, &user, "duplicate two").await;
    let valid = common::legacy_group(&db, &user, "valid").await;
    let missing = common::legacy_group(&db, &user, "missing event").await;
    sqlx::query("UPDATE models_group SET group_unique_global_id = ? WHERE group_id = ?")
        .bind(&first.unique_global_id)
        .bind(second.id)
        .execute(&db)
        .await
        .unwrap();
    for id in [first.id, second.id, valid.id, missing.id] {
        sqlx::query(
            "UPDATE models_group SET group_deleted_at = '2099-01-01T00:00:00Z' WHERE group_id = ?",
        )
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    }
    let ambiguous_event = random_hex_32();
    let valid_event = random_hex_32();
    for (global, event) in [
        (&first.unique_global_id, &ambiguous_event),
        (&valid.unique_global_id, &valid_event),
    ] {
        sqlx::query("INSERT INTO group_deletion_records (user_id, group_global_id, operation_id, deleted_at, group_revision) VALUES (?, ?, ?, '2099-01-01T00:00:00Z', 0)")
            .bind(user.id).bind(global).bind(event).execute(&db).await.unwrap();
    }
    db.close().await;
    let upgraded = db::db_context::setup_db(&path, &dir.path().join("backups")).await;
    let items = group_db::get_trashed_groups(&upgraded, &user, 30)
        .await
        .unwrap();
    assert_eq!(items.len(), 4);
    for id in [first.id, second.id, missing.id] {
        let item = items.iter().find(|item| item.id == id).unwrap();
        assert!(
            item.deletion_operation_id.is_none(),
            "ambiguous or missing event must be visibly non-restorable"
        );
    }
    let known = items.iter().find(|item| item.id == valid.id).unwrap();
    assert_eq!(
        known.deletion_operation_id.as_deref(),
        Some(valid_event.as_str())
    );
    let valid_request = GroupRestoreRequest {
        id: valid.id,
        group_global_id: valid.unique_global_id.clone(),
        deletion_operation_id: valid_event,
    };
    let forged = GroupRestoreRequest {
        id: first.id,
        group_global_id: first.unique_global_id.clone(),
        deletion_operation_id: ambiguous_event,
    };
    assert!(
        group_db::restore_groups(&upgraded, &user, &[valid_request.clone(), forged], 30)
            .await
            .is_err()
    );
    let missing_request = GroupRestoreRequest {
        id: missing.id,
        group_global_id: missing.unique_global_id,
        deletion_operation_id: random_hex_32(),
    };
    assert!(
        group_db::restore_groups(&upgraded, &user, &[valid_request, missing_request], 30)
            .await
            .is_err()
    );
    let still_deleted: Option<String> =
        sqlx::query_scalar("SELECT group_deleted_at FROM models_group WHERE group_id = ?")
            .bind(valid.id)
            .fetch_one(&upgraded)
            .await
            .unwrap();
    assert!(
        still_deleted.is_some(),
        "ambiguous restore must roll back every item"
    );
}
