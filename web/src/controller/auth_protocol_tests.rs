//! Session checks for logins against recreated accounts.

use std::sync::{Arc, atomic::Ordering};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use axum_login::{AuthUser as _, AuthnBackend as _};
use tokio::sync::Notify;
use tower::ServiceExt;

use crate::user::{Credentials, TokenCredentials};

async fn app() -> (
    tempfile::TempDir,
    service::AppState,
    axum::Router,
    String,
    String,
) {
    let (dir, state) = crate::test_support::app_state().await;
    let password = crate::test_support::add_login_user(&state).await;
    let user = db::user_db::get_user_by_email(&state.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let token = db::random_hex_32();
    db::user_db::set_user_sync_token(&state.db, user.id, &token, "https://example.test", false)
        .await
        .unwrap();
    let router = super::login_router().merge(super::router()).merge(
        crate::controller::group_controller::router().layer(axum::middleware::from_fn(
            crate::app::update_session_middleware,
        )),
    );
    let router = crate::test_support::with_session(router, state.clone()).await;
    (dir, state, router, password, token)
}

fn post_json(path: &str, body: &serde_json::Value, cookie: Option<&str>) -> Request<Body> {
    let mut request = Request::post(path).header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }

    request.body(Body::from(body.to_string())).unwrap()
}

fn cookie(response: &axum::response::Response) -> String {
    response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn old_sessions_cannot_access_recreated_user_with_same_id_and_sync_url() {
    let (_dir, state, router, password, _) = app().await;
    let login = || {
        post_json(
            "/api/v1/login/password",
            &serde_json::json!({"email":"alice@example.com", "password":password}),
            None,
        )
    };
    let read_login = router.clone().oneshot(login()).await.unwrap();
    let write_login = router.clone().oneshot(login()).await.unwrap();
    assert_eq!(read_login.status(), StatusCode::NO_CONTENT);
    assert_eq!(write_login.status(), StatusCode::NO_CONTENT);
    let read_cookie = cookie(&read_login);
    let write_cookie = cookie(&write_login);
    let original = db::user_db::get_user_by_email(&state.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let password_hash: String =
        sqlx::query_scalar("SELECT user_password_hash FROM users WHERE user_id = ?")
            .bind(original.id)
            .fetch_one(&*state.db)
            .await
            .unwrap();
    db::user_db::delete_user(&state.db, original.id)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (user_id, user_name, user_email, user_password_hash, user_created_at) VALUES (?, 'replacement', 'replacement@example.com', ?, '2020-01-01T00:00:00Z')")
        .bind(original.id).bind(password_hash).execute(&*state.db).await.unwrap();
    db::user_db::set_user_sync_token(
        &state.db,
        original.id,
        &db::random_hex_32(),
        "https://example.test",
        false,
    )
    .await
    .unwrap();
    let replacement = db::user_db::get_user_by_id(&state.db, original.id)
        .await
        .unwrap()
        .unwrap();
    db::group_db::add_empty_group(&state.db, &replacement, "replacement secret", None)
        .await
        .unwrap();
    let read = Request::get("/api/v1/users/me")
        .header(header::COOKIE, read_cookie)
        .body(Body::empty())
        .unwrap();
    let read_response = router.clone().oneshot(read).await.unwrap();
    let write_response = router
        .clone()
        .oneshot(post_json(
            "/api/v1/groups",
            &serde_json::json!({"group_name":"unauthorized write"}),
            Some(&write_cookie),
        ))
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_name = 'unauthorized write'")
        .bind(original.id).fetch_one(&*state.db).await.unwrap();
    assert!(
        !read_response.status().is_success(),
        "old session read replacement account: {}",
        read_response.status()
    );
    assert!(
        !write_response.status().is_success(),
        "old session wrote replacement account: {}",
        write_response.status()
    );
    assert_eq!(count, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn login_cannot_bind_old_credentials_to_replacement_account() {
    let (dir, state, _, _, token) = app().await;
    let user = db::user_db::get_user_by_email(&state.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let hash: String = sqlx::query_scalar("SELECT user_password_hash FROM users WHERE user_id = ?")
        .bind(user.id)
        .fetch_one(&*state.db)
        .await
        .unwrap();
    let reached = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .after_release({
            let (reached, resume, armed, releases) = (
                reached.clone(),
                resume.clone(),
                armed.clone(),
                releases.clone(),
            );
            move |_, _| {
                let (reached, resume, armed, releases) = (
                    reached.clone(),
                    resume.clone(),
                    armed.clone(),
                    releases.clone(),
                );
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
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(dir.path().join("db.sqlite"))
                .foreign_keys(true),
        )
        .await
        .unwrap();
    let connection = pool.acquire().await.unwrap();
    armed.store(true, Ordering::SeqCst);
    drop(connection);
    let backend = crate::user::Backend::new(Arc::new(pool));
    let login_backend = backend.clone();
    let task = tokio::spawn(async move {
        login_backend
            .authenticate(Credentials::Token(TokenCredentials { token }))
            .await
            .unwrap()
            .unwrap()
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), reached.notified())
        .await
        .expect("authentication did not reach connection release");
    db::user_db::delete_user(&state.db, user.id).await.unwrap();
    sqlx::query("INSERT INTO users (user_id, user_name, user_email, user_password_hash, user_created_at) VALUES (?, 'replacement', 'replacement@example.com', ?, '2020-01-01T00:00:00Z')")
        .bind(user.id).bind(hash).execute(&*state.db).await.unwrap();
    db::user_db::set_user_sync_token(
        &state.db,
        user.id,
        &db::random_hex_32(),
        "https://example.test",
        false,
    )
    .await
    .unwrap();
    resume.notify_one();
    let authenticated = tokio::time::timeout(std::time::Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap();
    let replacement = backend.get_user(&user.id).await.unwrap().unwrap();
    assert_ne!(
        authenticated.session_auth_hash(),
        replacement.session_auth_hash(),
        "old credentials must not bind to replacement account identity"
    );
}
