//! A group created without models is a destination and must survive startup cleanup.

mod common;

use db::{group_db, model::user::User};
use service::export_service;

#[tokio::test]
async fn startup_cleanup_preserves_an_owned_empty_group() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let group = group_db::add_empty_group(&app_state.db, &user, "Empty destination", None)
        .await
        .unwrap();

    export_service::cleanup_library_on_startup(&app_state)
        .await
        .unwrap();

    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM models_group WHERE group_id = ?)")
            .bind(group.id)
            .fetch_one(&*app_state.db)
            .await
            .unwrap();
    assert!(exists, "startup cleanup must preserve a user's empty group");
}
