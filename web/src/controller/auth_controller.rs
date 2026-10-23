use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};

use db::user_db;
use service::AppState;

use crate::{
    error::ApplicationError,
    user::{AuthSession, Credentials, CurrentUser, PasswordCredentials, TokenCredentials},
};

/// The credential checks, which the caller rate-limits against guessing.
pub fn login_router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/login/password", post(password))
            .route("/login/token", post(token)),
    )
}

/// Session routes. Kept out of the login rate limit: every page load calls `/users/me`, and a
/// 429 there reads as logged out.
pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/users/me", get(me))
            .route("/logout", post(logout)),
    )
}

pub async fn me(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
) -> Result<Response, ApplicationError> {
    let Some(user) = user_db::get_user_by_id(&app_state.db, user.id).await? else {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    };

    Ok(Json(user).into_response())
}

pub async fn password(
    auth_session: AuthSession,
    Json(credentials): Json<PasswordCredentials>,
) -> Response {
    login_inner(
        auth_session,
        Credentials::Password(credentials),
        "Invalid username or password",
    )
    .await
}

pub async fn token(
    auth_session: AuthSession,
    Json(credentials): Json<TokenCredentials>,
) -> Response {
    login_inner(
        auth_session,
        Credentials::Token(credentials),
        "Invalid token",
    )
    .await
}

/// Shared login flow: authenticate, then start the session. Only the 401
/// message differs between the password and token endpoints, so it is a parameter.
async fn login_inner(
    mut auth_session: AuthSession,
    credentials: Credentials,
    invalid_message: &'static str,
) -> Response {
    let user = match auth_session.authenticate(credentials).await {
        Ok(Some(user)) => user,
        Ok(None) => return (StatusCode::UNAUTHORIZED, invalid_message).into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    if auth_session.login(&user).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::NO_CONTENT.into_response()
}

pub async fn logout(mut auth_session: AuthSession) -> impl IntoResponse {
    if auth_session.logout().await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    /// Whether `router` has a route for `method path`. A held route answers anything but 404:
    /// with no auth layer installed, its session extractor fails with a 500.
    async fn holds_route(
        router: axum::Router<service::AppState>,
        method: axum::http::Method,
        path: &str,
    ) -> bool {
        // The pool is lazy and never connects: no request gets past the session extractor.
        let app_state = service::AppState {
            db: std::sync::Arc::new(sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap()),
            configuration: std::sync::Mutex::new(service::Configuration::default()),
            import_mutex: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            app_data_path: String::new(),
        };
        let request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .body(axum::body::Body::empty())
            .unwrap();
        let response = tower::ServiceExt::oneshot(router.with_state(app_state), request)
            .await
            .unwrap();

        response.status() != axum::http::StatusCode::NOT_FOUND
    }

    #[tokio::test]
    async fn login_router_holds_only_the_credential_checks_and_router_the_session_routes() {
        // The caller rate-limits all of `login_router`; `/users/me` runs on every page load, so a
        // 429 there would read as logged out.
        for (method, path, is_credential_check) in [
            (axum::http::Method::POST, "/api/v1/login/password", true),
            (axum::http::Method::POST, "/api/v1/login/token", true),
            (axum::http::Method::GET, "/api/v1/users/me", false),
            (axum::http::Method::POST, "/api/v1/logout", false),
        ] {
            let in_login_router = holds_route(super::login_router(), method.clone(), path).await;
            let in_router = holds_route(super::router(), method, path).await;

            assert_eq!(
                (in_login_router, in_router),
                (is_credential_check, !is_credential_check),
                "{path}"
            );
        }
    }
}
