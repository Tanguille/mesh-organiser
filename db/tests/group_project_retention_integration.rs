use db::{group_db, model_db, resource_db};

mod common;

#[tokio::test]
async fn project_survives_when_every_group_model_is_trashed_and_restored() {
    let (_dir, db) = common::test_db().await;
    let user = db::model::user::User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let project = resource_db::add_resource(&db, &user, "project", None)
        .await
        .unwrap();
    let blob = db::blob_db::add_blob(&db, &db::random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "model", blob, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    resource_db::set_resource_on_group(&db, &user, Some(project.id), group.id, None)
        .await
        .unwrap();
    model_db::trash_models(&db, &user, &[model]).await.unwrap();
    group_db::delete_dead_groups(&db).await.unwrap();
    let during: (Option<i64>,Option<i64>) = sqlx::query_as("SELECT g.group_resource_id,m.model_group_id FROM models_group g JOIN models m ON m.model_id=? WHERE g.group_id=? AND g.group_user_id=?").bind(model).bind(group.id).bind(user.id).fetch_one(&db).await.unwrap();
    assert_eq!(during, (Some(project.id), Some(group.id)));
    model_db::restore_models(&db, &user, &[model])
        .await
        .unwrap();
    let after: (Option<i64>,Option<i64>) = sqlx::query_as("SELECT g.group_resource_id,m.model_group_id FROM models_group g JOIN models m ON m.model_id=? WHERE g.group_id=? AND g.group_user_id=?").bind(model).bind(group.id).bind(user.id).fetch_one(&db).await.unwrap();
    assert_eq!(after, (Some(project.id), Some(group.id)));
}
