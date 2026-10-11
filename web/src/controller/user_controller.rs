use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use axum_login::login_required;
use serde::{Deserialize, Serialize};

use db::{
    model::user::{User, UserPermissions},
    user_db,
};
use service::{AppState, export_service};

use crate::{
    error::ApplicationError,
    user::{Backend, CurrentUser},
};

/// Rejects the request unless the caller has the Admin permission.
fn require_admin(user: &User, action: &str) -> Result<(), ApplicationError> {
    if !user.permissions.contains(UserPermissions::Admin) {
        return Err(ApplicationError::InternalError(format!(
            "Insufficient permissions to {action}."
        )));
    }

    Ok(())
}

/// Rejects the request unless the caller is an Admin or is acting on their own account.
fn require_admin_or_self(
    user: &User,
    target_id: i64,
    action: &str,
) -> Result<(), ApplicationError> {
    if user.id == target_id {
        return Ok(());
    }

    require_admin(user, action)
}

/// Routes for the user management endpoints; `login_required!` guards only those registered before it.
pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/users", get(get_users))
            .route("/users", post(add_user))
            .route("/users/{user_id}", put(edit_user))
            .route("/users/{user_id}", delete(delete_user))
            .route("/users/{user_id}/token", delete(generate_new_sync_token))
            .route("/users/{user_id}/password", put(edit_user_password))
            .route("/users/{user_id}/permissions", put(edit_user_permissions))
            .route_layer(login_required!(Backend)),
    )
}

pub async fn get_users(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
) -> Result<Response, ApplicationError> {
    require_admin(&user, "view users")?;

    let users = user_db::get_users(&app_state.db).await?;

    Ok(Json(users).into_response())
}

#[derive(Deserialize)]
#[allow(clippy::struct_field_names)] // field names match API
pub struct PostUserParams {
    pub user_name: String,
    pub user_email: String,
    pub user_password: String,
}

#[derive(Serialize)]
pub struct PostUserResponse {
    pub id: i64,
}

pub async fn add_user(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
    Json(params): Json<PostUserParams>,
) -> Result<Json<PostUserResponse>, ApplicationError> {
    require_admin(&user, "add a new user")?;

    let id = user_db::add_user(
        &app_state.db,
        &params.user_name,
        &params.user_email,
        &params.user_password,
    )
    .await?;

    user_db::scramble_validity_token(&app_state.db, id).await?;

    Ok(Json(PostUserResponse { id }))
}

#[derive(Deserialize)]
pub struct PutUserParams {
    pub user_name: String,
    pub user_email: String,
}

pub async fn edit_user(
    CurrentUser(user): CurrentUser,
    Path(user_id): Path<i64>,
    State(app_state): State<AppState>,
    Json(params): Json<PutUserParams>,
) -> Result<StatusCode, ApplicationError> {
    require_admin_or_self(&user, user_id, "change this user's password")?;

    user_db::edit_user_min(
        &app_state.db,
        user_id,
        &params.user_name,
        &params.user_email,
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct PutUserPasswordParams {
    pub new_password: String,
}

pub async fn edit_user_password(
    CurrentUser(user): CurrentUser,
    Path(user_id): Path<i64>,
    State(app_state): State<AppState>,
    Json(params): Json<PutUserPasswordParams>,
) -> Result<StatusCode, ApplicationError> {
    require_admin_or_self(&user, user_id, "change this user's password")?;

    user_db::edit_user_password(&app_state.db, user_id, &params.new_password).await?;

    user_db::scramble_validity_token(&app_state.db, user_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct PutUserPermissionsParams {
    pub permissions: UserPermissions,
}

pub async fn edit_user_permissions(
    CurrentUser(user): CurrentUser,
    Path(user_id): Path<i64>,
    State(app_state): State<AppState>,
    Json(params): Json<PutUserPermissionsParams>,
) -> Result<StatusCode, ApplicationError> {
    require_admin(&user, "change user permissions")?;

    user_db::set_user_permissions(&app_state.db, user_id, params.permissions).await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_user(
    CurrentUser(user): CurrentUser,
    Path(user_id): Path<i64>,
    State(app_state): State<AppState>,
) -> Result<StatusCode, ApplicationError> {
    require_admin_or_self(&user, user_id, "delete this user")?;

    user_db::delete_user(&app_state.db, user_id).await?;

    export_service::delete_dead_blobs(&app_state).await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn generate_new_sync_token(
    CurrentUser(user): CurrentUser,
    Path(user_id): Path<i64>,
    State(app_state): State<AppState>,
) -> Result<StatusCode, ApplicationError> {
    require_admin_or_self(&user, user_id, "generate a new sync token for this user")?;

    user_db::scramble_login_token(&app_state.db, user_id).await?;

    Ok(StatusCode::NO_CONTENT)
}
