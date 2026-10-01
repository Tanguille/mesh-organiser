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

pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/login/password", post(password))
            .route("/login/token", post(token))
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
    Json(creds): Json<PasswordCredentials>,
) -> Response {
    login_inner(
        auth_session,
        Credentials::Password(creds),
        "Invalid username or password",
    )
    .await
}

pub async fn token(auth_session: AuthSession, Json(creds): Json<TokenCredentials>) -> Response {
    login_inner(auth_session, Credentials::Token(creds), "Invalid token").await
}

async fn login_inner(
    mut auth_session: AuthSession,
    creds: Credentials,
    invalid_message: &'static str,
) -> Response {
    let user = match auth_session.authenticate(creds).await {
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
