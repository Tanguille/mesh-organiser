//! Portable birth ids identify new logical entities without inventing legacy ancestry.
mod common;
use db::{
    blob_db,
    db_context::DbContext,
    group_db,
    model::{ModelFlags, user::User},
    model_db, random_hex_32, user_db,
};

async fn assert_schema(db: &DbContext) {
    for (table, column) in [
        ("models_group", "group_birth_id"),
        ("models", "model_birth_id"),
    ] {
        let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?)")
            .bind(table)
            .fetch_all(db)
            .await
            .unwrap();
        assert!(
            columns.iter().any(|name| name == column),
            "missing {table}.{column}"
        );
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'entity_births'",
    )
    .fetch_one(db)
    .await
    .unwrap();
    assert_eq!(count, 1, "missing durable entity birth registry");
}
async fn birth(db: &DbContext, table: &str, id: i64) -> Option<String> {
    match table {
        "group" => sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap(),
        "model" => sqlx::query_scalar("SELECT model_birth_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap(),
        _ => unreachable!(),
    }
}
fn valid_birth(value: &str) {
    assert_eq!(value.len(), 32);
    assert!(
        value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
}
async fn registry_global(db: &DbContext, user: i64, kind: &str, value: &str) -> Option<String> {
    sqlx::query_scalar("SELECT global_id FROM entity_births WHERE user_id = ? AND entity_kind = ? AND birth_id = ?").bind(user).bind(kind).bind(value).fetch_optional(db).await.unwrap()
}

#[tokio::test]
async fn new_births_are_owner_scoped_stable_immutable_and_survive_purge() {
    let (dir, db) = common::test_db().await;
    assert_schema(&db).await;
    let owner = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &owner, "folder", None)
        .await
        .unwrap();
    let foreign = group_db::add_empty_group(&db, &other, "foreign", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, "birth-model", "stl", 1, None)
        .await
        .unwrap();
    let id = model_db::add_model(&db, &owner, "part", blob, None, None)
        .await
        .unwrap();
    let model_global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    let gb = birth(&db, "group", group.id).await.unwrap();
    let mb = birth(&db, "model", id).await.unwrap();
    let fb = birth(&db, "group", foreign.id).await.unwrap();
    for value in [&gb, &mb, &fb] {
        valid_birth(value);
    }
    assert_ne!(gb, fb);
    assert_eq!(
        registry_global(&db, owner.id, "group", &gb).await,
        Some(group.unique_global_id.clone())
    );
    assert_eq!(
        registry_global(&db, owner.id, "model", &mb).await,
        Some(model_global)
    );
    assert_eq!(registry_global(&db, other.id, "group", &gb).await, None);
    group_db::edit_group(&db, &owner, group.id, "renamed", None, None)
        .await
        .unwrap();
    assert_eq!(
        birth(&db, "group", group.id).await.as_deref(),
        Some(gb.as_str())
    );
    assert!(
        sqlx::query("UPDATE models_group SET group_birth_id = ? WHERE group_id = ?")
            .bind(random_hex_32())
            .bind(group.id)
            .execute(&db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE models SET model_birth_id = ? WHERE model_id = ?")
            .bind(random_hex_32())
            .bind(id)
            .execute(&db)
            .await
            .is_err()
    );
    db.close().await;
    let db =
        db::db_context::setup_db(&dir.path().join("db.sqlite"), &dir.path().join("backups")).await;
    assert_eq!(
        birth(&db, "group", group.id).await.as_deref(),
        Some(gb.as_str())
    );
    assert_eq!(birth(&db, "model", id).await.as_deref(), Some(mb.as_str()));
    group_db::delete_group(&db, &owner, group.id).await.unwrap();
    model_db::trash_models(&db, &owner, &[id]).await.unwrap();
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = '2000-01-01T00:00:00Z' WHERE group_id = ?",
    )
    .bind(group.id)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query("UPDATE models SET model_deleted = '2000-01-01T00:00:00Z' WHERE model_id = ?")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    group_db::purge_trashed_groups(&db, 30).await.unwrap();
    model_db::purge_trashed_models(&db, 30).await.unwrap();
    assert_eq!(
        registry_global(&db, owner.id, "group", &gb).await,
        Some(group.unique_global_id)
    );
    assert!(registry_global(&db, owner.id, "model", &mb).await.is_some());
}

#[tokio::test]
async fn rekey_before_history_updates_registry_and_preserves_birth() {
    let (_dir, db) = common::test_db().await;
    assert_schema(&db).await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, "rekey-model", "stl", 1, None)
        .await
        .unwrap();
    let id = model_db::add_model(&db, &user, "part", blob, None, None)
        .await
        .unwrap();
    let gb = birth(&db, "group", group.id).await.unwrap();
    let mb = birth(&db, "model", id).await.unwrap();
    let new_group = random_hex_32();
    let new_model = random_hex_32();
    group_db::edit_group(&db, &user, group.id, "folder", None, Some(&new_group))
        .await
        .unwrap();
    model_db::edit_model(
        &db,
        &user,
        id,
        "part",
        None,
        None,
        ModelFlags::empty(),
        None,
        Some(&new_model),
    )
    .await
    .unwrap();
    assert_eq!(
        registry_global(&db, user.id, "group", &gb).await,
        Some(new_group)
    );
    assert_eq!(
        registry_global(&db, user.id, "model", &mb).await,
        Some(new_model)
    );
    assert_eq!(birth(&db, "group", group.id).await, Some(gb));
    assert_eq!(birth(&db, "model", id).await, Some(mb));
}

#[tokio::test]
async fn failed_registry_insert_rolls_back_new_entities() {
    let (_dir, db) = common::test_db().await;
    assert_schema(&db).await;
    let user = User::default();
    let blob = blob_db::add_blob(&db, "failure-model", "stl", 1, None)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER reject_birth BEFORE INSERT ON entity_births BEGIN SELECT RAISE(ABORT, 'birth failure'); END").execute(&db).await.unwrap();
    assert!(
        group_db::add_empty_group(&db, &user, "failure", None)
            .await
            .is_err()
    );
    assert!(
        model_db::add_model(&db, &user, "failure", blob, None, None)
            .await
            .is_err()
    );
    let (groups, models): (i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group WHERE group_name = 'failure'), (SELECT COUNT(*) FROM models WHERE model_name = 'failure')").fetch_one(&db).await.unwrap();
    assert_eq!((groups, models), (0, 0));
}

#[tokio::test]
async fn migration_leaves_old_group_and_model_births_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let migrations = sqlx::migrate!("./migrations")
        .iter()
        .filter(|migration| migration.version < 20_261_112_230_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(migrations)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let global = random_hex_32();
    let group = sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id) VALUES ('legacy', '2020-01-01T00:00:00Z', ?, '2020-01-01T00:00:00Z', ?)")
        .bind(user.id).bind(global).execute(&db).await.unwrap().last_insert_rowid();
    let blob = blob_db::add_blob(&db, "legacy-birth", "stl", 1, None)
        .await
        .unwrap();
    let model = sqlx::query("INSERT INTO models (model_name, model_blob_id, model_added, model_user_id, model_unique_global_id, model_last_modified) VALUES ('legacy', ?, '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')")
        .bind(blob).bind(user.id).bind(random_hex_32()).execute(&db).await.unwrap().last_insert_rowid();
    db.close().await;
    let db = db::db_context::setup_db(&path, &dir.path().join("backups")).await;
    assert_schema(&db).await;
    assert_eq!(birth(&db, "group", group).await, None);
    assert_eq!(birth(&db, "model", model).await, None);
    group_db::edit_group(&db, &user, group, "legacy renamed", None, None)
        .await
        .unwrap();
    assert_eq!(
        birth(&db, "group", group).await,
        None,
        "local edits cannot infer legacy birth"
    );
}

#[tokio::test]
async fn inserting_duplicate_live_birth_for_an_owner_is_rejected() {
    let (_dir, db) = common::test_db().await;
    assert_schema(&db).await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "first", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, "duplicate-birth", "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "first", blob, None, None)
        .await
        .unwrap();
    let gb = birth(&db, "group", group.id).await.unwrap();
    let mb = birth(&db, "model", model).await.unwrap();
    let group_result = sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id, group_birth_id) VALUES ('duplicate', '2020-01-01T00:00:00Z', ?, '2020-01-01T00:00:00Z', ?, ?)")
        .bind(user.id).bind(random_hex_32()).bind(gb).execute(&db).await;
    assert!(group_result.is_err());
    let model_result = sqlx::query("INSERT INTO models (model_name, model_blob_id, model_added, model_user_id, model_unique_global_id, model_last_modified, model_birth_id) VALUES ('duplicate', ?, '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z', ?)")
        .bind(blob).bind(user.id).bind(random_hex_32()).bind(mb).execute(&db).await;
    assert!(model_result.is_err());
}
