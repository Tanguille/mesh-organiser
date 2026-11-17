use db::db_context;

#[allow(dead_code)]
pub async fn test_db() -> (tempfile::TempDir, db_context::DbContext) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("db.sqlite");
    let backup_dir = dir.path().join("backups");
    std::fs::create_dir_all(&backup_dir).unwrap();
    let db = db_context::setup_db(&db_path, &backup_dir).await;

    (dir, db)
}

#[allow(dead_code)]
pub async fn legacy_group(
    db: &db_context::DbContext,
    user: &db::model::user::User,
    name: &str,
) -> db::model::model_group::ModelGroupMeta {
    let global = db::random_hex_32();
    let timestamp = "2020-01-01T00:00:00Z";
    let id = sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id) VALUES (?, ?, ?, ?, ?)")
        .bind(name).bind(timestamp).bind(user.id).bind(timestamp).bind(&global).execute(db).await.unwrap().last_insert_rowid();
    db::model::model_group::ModelGroupMeta {
        id,
        name: name.into(),
        created: timestamp.into(),
        last_modified: timestamp.into(),
        resource_id: None,
        unique_global_id: global,
    }
}

#[allow(dead_code)]
pub async fn legacy_model(
    db: &db_context::DbContext,
    user: &db::model::user::User,
    name: &str,
) -> i64 {
    let blob = db::blob_db::add_blob(db, name, "stl", 1, None)
        .await
        .unwrap();
    sqlx::query("INSERT INTO models (model_name, model_blob_id, model_added, model_user_id, model_unique_global_id, model_last_modified) VALUES (?, ?, '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')")
        .bind(name).bind(blob).bind(user.id).bind(db::random_hex_32()).execute(db).await.unwrap().last_insert_rowid()
}

#[allow(dead_code)]
pub async fn group_restore_request(
    db: &db_context::DbContext,
    user: &db::model::user::User,
    id: i64,
) -> db::group_db::GroupRestoreRequest {
    let group = db::group_db::get_trashed_groups(db, user, 30)
        .await
        .unwrap()
        .into_iter()
        .find(|group| group.id == id)
        .unwrap();
    db::group_db::GroupRestoreRequest {
        id,
        group_global_id: group.group_global_id,
        deletion_operation_id: group.deletion_operation_id.unwrap(),
    }
}
