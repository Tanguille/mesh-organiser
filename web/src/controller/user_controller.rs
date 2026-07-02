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
use service::export_service;

use crate::{
    error::ApplicationError,
    user::{AuthSession, Backend},
    web_app_state::WebAppState,
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
    if !user.permissions.contains(UserPermissions::Admin) && user.id != target_id {
        return Err(ApplicationError::InternalError(format!(
            "Insufficient permissions to {action}."
        )));
    }

    Ok(())
}

pub fn router() -> Router<WebAppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/users", get(get::get_users))
            .route("/users", post(post::add_user))
            .route("/users/{user_id}", put(put::edit_user))
            .route("/users/{user_id}", delete(delete::delete_user))
            .route(
                "/users/{user_id}/token",
                delete(delete::generate_new_sync_token),
            )
            .route("/users/{user_id}/password", put(put::edit_user_password))
            .route(
                "/users/{user_id}/permissions",
                put(put::edit_user_permissions),
            )
            .route_layer(login_required!(Backend)),
    )
}

mod get {
    use super::{
        ApplicationError, AuthSession, IntoResponse, Json, Response, State, WebAppState,
        require_admin, user_db,
    };

    pub async fn get_users(
        auth_session: AuthSession,
        State(app_state): State<WebAppState>,
    ) -> Result<Response, ApplicationError> {
        let user = auth_session.user.unwrap().to_user();

        require_admin(&user, "view users")?;

        let users = user_db::get_users(&app_state.app_state.db).await?;

        Ok(Json(users).into_response())
    }
}

mod post {
    use super::{
        ApplicationError, AuthSession, Deserialize, IntoResponse, Json, Response, Serialize, State,
        WebAppState, require_admin, user_db,
    };

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
        auth_session: AuthSession,
        State(app_state): State<WebAppState>,
        Json(params): Json<PostUserParams>,
    ) -> Result<Response, ApplicationError> {
        let user = auth_session.user.unwrap().to_user();

        require_admin(&user, "add a new user")?;

        let id = user_db::add_user(
            &app_state.app_state.db,
            &params.user_name,
            &params.user_email,
            &params.user_password,
        )
        .await?;

        user_db::scramble_validity_token(&app_state.app_state.db, id).await?;

        Ok(Json(PostUserResponse { id }).into_response())
    }
}

mod put {
    use super::{
        ApplicationError, AuthSession, Deserialize, IntoResponse, Json, Path, Response, State,
        StatusCode, UserPermissions, WebAppState, require_admin, require_admin_or_self, user_db,
    };

    #[derive(Deserialize)]
    pub struct PutUserParams {
        pub user_name: String,
        pub user_email: String,
    }

    pub async fn edit_user(
        auth_session: AuthSession,
        Path(user_id): Path<i64>,
        State(app_state): State<WebAppState>,
        Json(params): Json<PutUserParams>,
    ) -> Result<Response, ApplicationError> {
        let user = auth_session.user.unwrap().to_user();

        require_admin_or_self(&user, user_id, "change this user's password")?;

        user_db::edit_user_min(
            &app_state.app_state.db,
            user_id,
            &params.user_name,
            &params.user_email,
        )
        .await?;

        Ok(StatusCode::NO_CONTENT.into_response())
    }

    #[derive(Deserialize)]
    pub struct PutUserPasswordParams {
        pub new_password: String,
    }

    pub async fn edit_user_password(
        auth_session: AuthSession,
        Path(user_id): Path<i64>,
        State(app_state): State<WebAppState>,
        Json(params): Json<PutUserPasswordParams>,
    ) -> Result<Response, ApplicationError> {
        let user = auth_session.user.unwrap().to_user();

        require_admin_or_self(&user, user_id, "change this user's password")?;

        user_db::edit_user_password(&app_state.app_state.db, user_id, &params.new_password).await?;

        user_db::scramble_validity_token(&app_state.app_state.db, user_id).await?;

        Ok(StatusCode::NO_CONTENT.into_response())
    }

    #[derive(Deserialize)]
    pub struct PutUserPermissionsParams {
        pub permissions: UserPermissions,
    }

    pub async fn edit_user_permissions(
        auth_session: AuthSession,
        Path(user_id): Path<i64>,
        State(app_state): State<WebAppState>,
        Json(params): Json<PutUserPermissionsParams>,
    ) -> Result<Response, ApplicationError> {
        let user = auth_session.user.unwrap().to_user();

        require_admin(&user, "change user permissions")?;

        user_db::set_user_permissions(&app_state.app_state.db, user_id, params.permissions).await?;

        Ok(StatusCode::NO_CONTENT.into_response())
    }
}

mod delete {
    use super::{
        ApplicationError, AuthSession, IntoResponse, Path, Response, State, StatusCode,
        WebAppState, export_service, require_admin_or_self, user_db,
    };

    pub async fn delete_user(
        auth_session: AuthSession,
        Path(user_id): Path<i64>,
        State(app_state): State<WebAppState>,
    ) -> Result<Response, ApplicationError> {
        let user = auth_session.user.unwrap().to_user();

        require_admin_or_self(&user, user_id, "delete this user")?;

        user_db::delete_user(&app_state.app_state.db, user_id).await?;

        export_service::delete_dead_blobs(&app_state.app_state).await?;

        Ok(StatusCode::NO_CONTENT.into_response())
    }

    pub async fn generate_new_sync_token(
        auth_session: AuthSession,
        Path(user_id): Path<i64>,
        State(app_state): State<WebAppState>,
    ) -> Result<Response, ApplicationError> {
        let user = auth_session.user.unwrap().to_user();

        require_admin_or_self(&user, user_id, "generate a new sync token for this user")?;

        user_db::scramble_login_token(&app_state.app_state.db, user_id).await?;

        Ok(StatusCode::NO_CONTENT.into_response())
    }
}
