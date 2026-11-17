//! Authentication and owner isolation for group Trash routes.

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use tower::ServiceExt;

use db::{group_db, model::user::User};

async fn app() -> (
    tempfile::TempDir,
    service::AppState,
    axum::Router,
    String,
    User,
) {
    let (dir, state) = crate::test_support::app_state().await;
    let password = crate::test_support::add_login_user(&state).await;
    let alice = db::user_db::get_user_by_email(&state.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let router = crate::test_support::with_session(
        crate::controller::auth_controller::login_router().merge(super::router()),
        state.clone(),
    )
    .await;
    let response = router
        .clone()
        .oneshot(crate::test_support::login_request(&password))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    (dir, state, router, cookie, alice)
}

fn request(method: &str, path: &str, cookie: Option<&str>, body: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }

    builder.body(Body::from(body.to_owned())).unwrap()
}

#[tokio::test]
async fn group_recovery_requires_authentication_and_has_no_public_share_route() {
    let (_dir, state, router, _, _) = app().await;
    for (method, path, body) in [
        ("GET", "/api/v1/groups/trash", ""),
        ("POST", "/api/v1/groups/restore", r#"{"groups":[]}"#),
    ] {
        let response = router
            .clone()
            .oneshot(request(method, path, None, body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
    let share = db::share_db::create_share(&state.db, &User::default(), "public")
        .await
        .unwrap();
    let path = format!("/api/v1/shares/{share}/groups/trash");
    let response = router
        .oneshot(request("GET", &path, None, ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn group_trash_lists_only_the_owner_and_rejects_foreign_restore() {
    let (_dir, state, router, cookie, alice) = app().await;
    let other = User::default();
    let own = group_db::add_empty_group(&state.db, &alice, "own deleted", None)
        .await
        .unwrap();
    let foreign = group_db::add_empty_group(&state.db, &other, "foreign deleted", None)
        .await
        .unwrap();
    group_db::delete_group(&state.db, &alice, own.id)
        .await
        .unwrap();
    group_db::delete_group(&state.db, &other, foreign.id)
        .await
        .unwrap();

    let response = router
        .clone()
        .oneshot(request("GET", "/api/v1/groups/trash", Some(&cookie), ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let items: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], own.id);
    assert_eq!(items[0]["name"], "own deleted");
    assert!(items[0]["deleted"].as_str().is_some());
    assert_eq!(items[0]["group_global_id"], own.unique_global_id);
    let own_operation = items[0]["deletion_operation_id"].as_str().unwrap();
    assert_ne!(own_operation, "");

    let foreign_operation: String = sqlx::query_scalar(
        "SELECT operation_id FROM group_deletion_records WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(other.id)
    .bind(&foreign.unique_global_id)
    .fetch_one(&*state.db)
    .await
    .unwrap();
    let body = serde_json::json!({ "groups": [{"id": foreign.id, "group_global_id": foreign.unique_global_id, "deletion_operation_id": foreign_operation}] }).to_string();
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/groups/restore",
            Some(&cookie),
            &body,
        ))
        .await
        .unwrap();
    assert!(
        !response.status().is_success(),
        "foreign restore must be rejected"
    );
    let foreign_deleted: Option<String> =
        sqlx::query_scalar("SELECT group_deleted_at FROM models_group WHERE group_id = ?")
            .bind(foreign.id)
            .fetch_one(&*state.db)
            .await
            .unwrap();
    assert!(foreign_deleted.is_some());

    let body = serde_json::json!({ "group_ids": [own.id] }).to_string();
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/groups/restore",
            Some(&cookie),
            &body,
        ))
        .await
        .unwrap();
    assert!(
        !response.status().is_success(),
        "id-only restore must be rejected"
    );
    let body = serde_json::json!({ "groups": [{"id": own.id, "group_global_id": own.unique_global_id, "deletion_operation_id": own_operation}] }).to_string();
    let response = router
        .oneshot(request(
            "POST",
            "/api/v1/groups/restore",
            Some(&cookie),
            &body,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let own_deleted: Option<String> =
        sqlx::query_scalar("SELECT group_deleted_at FROM models_group WHERE group_id = ?")
            .bind(own.id)
            .fetch_one(&*state.db)
            .await
            .unwrap();
    assert!(own_deleted.is_none());
}

#[tokio::test]
async fn restore_rejects_a_stale_deletion_identity_after_redelete() {
    let (_dir, state, router, cookie, user) = app().await;
    let group = group_db::add_empty_group(&state.db, &user, "repeated", None)
        .await
        .unwrap();
    group_db::delete_group(&state.db, &user, group.id)
        .await
        .unwrap();
    let first: String = sqlx::query_scalar("SELECT operation_id FROM group_deletion_records WHERE user_id = ? AND group_global_id = ? AND restored_at IS NULL")
        .bind(user.id).bind(&group.unique_global_id).fetch_one(&*state.db).await.unwrap();
    let body = serde_json::json!({ "groups": [{"id": group.id, "group_global_id": group.unique_global_id, "deletion_operation_id": first}] }).to_string();
    let restore = |body: &str| request("POST", "/api/v1/groups/restore", Some(&cookie), body);
    assert_eq!(
        router
            .clone()
            .oneshot(restore(&body))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    group_db::delete_group(&state.db, &user, group.id)
        .await
        .unwrap();
    let response = router.clone().oneshot(restore(&body)).await.unwrap();
    assert!(
        !response.status().is_success(),
        "stale deletion identity must be rejected"
    );
    let response = router
        .oneshot(request("GET", "/api/v1/groups/trash", Some(&cookie), ""))
        .await
        .unwrap();
    let raw = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&raw).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], group.id);
    assert_ne!(entries[0]["deletion_operation_id"], first);
}
