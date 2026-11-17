//! Sync snapshots must be authenticated, account-pinned and lossless on the wire.
use std::sync::{Arc, atomic::Ordering};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use tokio::sync::Notify;
use tower::ServiceExt;

use db::{group_db, model::user::User, user_db};

async fn fixture() -> (
    tempfile::TempDir,
    service::AppState,
    axum::Router,
    String,
    User,
) {
    let (dir, state) = crate::test_support::app_state().await;
    let password = crate::test_support::add_login_user(&state).await;
    let user = user_db::get_user_by_email(&state.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let router = crate::test_support::with_session(
        crate::controller::auth_controller::login_router().merge(super::router()),
        state.clone(),
    )
    .await;
    let login = router
        .clone()
        .oneshot(crate::test_support::login_request(&password))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::NO_CONTENT);
    let cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    (dir, state, router, cookie, user)
}

fn request(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut req = Request::get(path);
    if let Some(cookie) = cookie {
        req = req.header(header::COOKIE, cookie);
    }

    req.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn snapshot_requires_session_and_is_not_available_from_public_share() {
    let (_dir, state, router, _, _) = fixture().await;
    let result = router
        .clone()
        .oneshot(request("/api/v2/sync/snapshot", None))
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::UNAUTHORIZED);
    let share = db::share_db::create_share(&state.db, &User::default(), "public")
        .await
        .unwrap();
    let path = format!("/api/v1/shares/{share}/sync/snapshot");
    assert_eq!(
        router
            .clone()
            .oneshot(request(&path, None))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let req = Request::get("/api/v2/sync/snapshot")
        .header(header::AUTHORIZATION, format!("Bearer {share}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router.oneshot(req).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn snapshot_is_owner_scoped_and_counters_are_decimal_strings() {
    let (_dir, state, router, cookie, user) = fixture().await;
    let own = group_db::add_empty_group(&state.db, &user, "own group", None)
        .await
        .unwrap();
    let foreign = group_db::add_empty_group(&state.db, &User::default(), "foreign secret", None)
        .await
        .unwrap();
    let blob = db::blob_db::add_blob(&state.db, &db::random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let model = db::model_db::add_model(&state.db, &user, "own model", blob, None, None)
        .await
        .unwrap();
    let foreign_model = db::model_db::add_model(
        &state.db,
        &User::default(),
        "foreign model",
        blob,
        None,
        None,
    )
    .await
    .unwrap();
    let foreign_model_global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(foreign_model)
            .fetch_one(&*state.db)
            .await
            .unwrap();
    let foreign_resource =
        db::resource_db::add_resource(&state.db, &User::default(), "foreign resource", None)
            .await
            .unwrap();
    let own_resource = db::resource_db::add_resource(&state.db, &user, "own resource", None)
        .await
        .unwrap();
    let model_global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id = ?")
            .bind(model)
            .fetch_one(&*state.db)
            .await
            .unwrap();
    let deleted = group_db::add_empty_group(&state.db, &user, "deleted group", None)
        .await
        .unwrap();
    group_db::delete_group(&state.db, &user, deleted.id)
        .await
        .unwrap();
    let huge: i64 = 9_007_199_254_740_993;
    sqlx::query("UPDATE models SET model_assignment_revision = ?, model_lifecycle_revision = ? WHERE model_id = ?").bind(huge).bind(huge).bind(model).execute(&*state.db).await.unwrap();
    sqlx::query("UPDATE group_deletion_records SET group_revision = ? WHERE user_id = ? AND group_global_id = ?").bind(huge).bind(user.id).bind(&deleted.unique_global_id).execute(&*state.db).await.unwrap();
    sqlx::query("UPDATE models_group SET group_revision = ? WHERE group_id = ?")
        .bind(huge)
        .bind(own.id)
        .execute(&*state.db)
        .await
        .unwrap();
    sqlx::query("UPDATE group_sync_heads SET generation = ?, head = ? WHERE user_id = ? AND group_global_id = ?")
        .bind(huge).bind(huge).bind(user.id).bind(&own.unique_global_id).execute(&*state.db).await.unwrap();
    let response = router
        .oneshot(request("/api/v2/sync/snapshot", Some(&cookie)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let identity = user_db::get_sync_identity(&state.db, user.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(body["identity"]["account_id"], identity.account_id);
    assert_eq!(body["identity"]["instance_id"], identity.instance_id);
    let groups = body["groups"].as_array().unwrap();
    let group = groups
        .iter()
        .find(|group| group["global_id"] == own.unique_global_id)
        .unwrap();
    assert_eq!(group["revision"], "9007199254740993");
    assert!(
        !groups
            .iter()
            .any(|group| group["global_id"] == foreign.unique_global_id)
    );
    let heads = body["heads"].as_array().unwrap();
    let head = heads
        .iter()
        .find(|head| head["group_global_id"] == own.unique_global_id)
        .unwrap();
    assert_eq!(head["head"], "9007199254740993");
    assert_eq!(head["generation"], "9007199254740993");
    let assignments = body["assignments"].as_array().unwrap();
    let assignment = assignments
        .iter()
        .find(|assignment| assignment["model_global_id"] == model_global)
        .unwrap();
    assert_eq!(assignment["revision"], "9007199254740993");
    assert_eq!(assignment["lifecycle_revision"], "9007199254740993");
    assert!(
        !assignments
            .iter()
            .any(|assignment| assignment["model_global_id"] == foreign_model_global)
    );
    let deletions = body["deletions"].as_array().unwrap();
    let deletion = deletions
        .iter()
        .find(|deletion| deletion["group_global_id"] == deleted.unique_global_id)
        .unwrap();
    assert_eq!(deletion["revision"], "9007199254740993");
    let resources = body["resources"].as_array().unwrap();
    assert!(
        resources
            .iter()
            .any(|resource| resource["global_id"] == own_resource.unique_global_id)
    );
    assert!(
        !resources
            .iter()
            .any(|resource| resource["global_id"] == foreign_resource.unique_global_id)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_rejects_account_replacement_after_authentication() {
    let (dir, state, _, cookie, user) = fixture().await;
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
                    if armed.load(Ordering::SeqCst) && releases.fetch_add(1, Ordering::SeqCst) == 2
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
    let mut request_state = state.clone();
    request_state.db = Arc::new(pool.clone());
    let router = crate::test_support::with_session(super::router(), request_state).await;
    let connection = pool.acquire().await.unwrap();
    armed.store(true, Ordering::SeqCst);
    drop(connection);
    let task = tokio::spawn(async move {
        router
            .oneshot(request("/api/v2/sync/snapshot", Some(&cookie)))
            .await
            .unwrap()
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), reached.notified())
        .await
        .expect("request did not reach authenticated account lookup");
    user_db::delete_user(&state.db, user.id).await.unwrap();
    sqlx::query("INSERT INTO users (user_id, user_name, user_email, user_password_hash, user_created_at) VALUES (?, 'replacement', 'replacement@example.com', ?, '2020-01-01T00:00:00Z')")
        .bind(user.id).bind(hash).execute(&*state.db).await.unwrap();
    let replacement = user_db::get_user_by_id(&state.db, user.id)
        .await
        .unwrap()
        .unwrap();
    group_db::add_empty_group(&state.db, &replacement, "replacement secret", None)
        .await
        .unwrap();
    resume.notify_one();
    let response = tokio::time::timeout(std::time::Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("replacement secret"));
}
