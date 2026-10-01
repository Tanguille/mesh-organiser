use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use axum_login::login_required;
use serde::Deserialize;

use db::{
    model::{
        share::{Share, ShareDto},
        user::User,
    },
    share_db, time_now, user_db,
};

use service::AppState;

use crate::{
    error::ApplicationError,
    user::{Backend, CurrentUser},
};

/// Resolves a share and its owning user, erroring if the owner no longer exists.
pub async fn resolve_share_owner(
    app_state: &AppState,
    share_id: &str,
) -> Result<(Share, User), ApplicationError> {
    let share = share_db::get_share_via_id(&app_state.db, share_id).await?;

    let Some(user) = user_db::get_user_by_id(&app_state.db, share.user_id).await? else {
        return Err(ApplicationError::InternalError(
            "Share owner user not found.".into(),
        ));
    };

    Ok((share, user))
}

pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/shares", get(get_shares))
            .route("/shares", post(create_share))
            .route("/shares/{share_id}", put(edit_share))
            .route("/shares/{share_id}", delete(delete_share))
            .route("/shares/{share_id}/models", put(set_model_ids_on_share))
            .route_layer(login_required!(Backend))
            .route("/shares/{share_id}", get(get_share)),
    )
}

pub async fn get_shares(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
) -> Result<Response, ApplicationError> {
    let shares = share_db::get_shares(&app_state.db, &user).await?;

    let shares: Vec<ShareDto> = shares
        .into_iter()
        .map(|s| s.to_dto(user.username.clone()))
        .collect();

    Ok(Json(shares).into_response())
}

pub async fn get_share(
    Path(share_id): Path<String>,
    State(app_state): State<AppState>,
) -> Result<Response, ApplicationError> {
    let (share, user) = resolve_share_owner(&app_state, &share_id).await?;

    let share = share.to_dto(user.username);

    Ok(Json(share).into_response())
}

#[derive(Deserialize)]
pub struct CreateShareParams {
    pub share_name: String,
}

pub async fn create_share(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
    Json(params): Json<CreateShareParams>,
) -> Result<Response, ApplicationError> {
    let share_id = share_db::create_share(&app_state.db, &user, &params.share_name).await?;

    Ok(Json(ShareDto {
        id: share_id,
        share_name: params.share_name,
        user_name: user.username,
        model_ids: Vec::new(),
        created_at: time_now(),
    })
    .into_response())
}

#[derive(Deserialize)]
pub struct EditShareParams {
    pub share_name: String,
}

pub async fn edit_share(
    CurrentUser(user): CurrentUser,
    Path(share_id): Path<String>,
    State(app_state): State<AppState>,
    Json(params): Json<EditShareParams>,
) -> Result<StatusCode, ApplicationError> {
    share_db::rename_share(&app_state.db, &user, &share_id, &params.share_name).await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn set_model_ids_on_share(
    CurrentUser(user): CurrentUser,
    Path(share_id): Path<String>,
    State(app_state): State<AppState>,
    Json(params): Json<crate::controller::ModelIdsParams>,
) -> Result<StatusCode, ApplicationError> {
    share_db::set_model_ids_on_share(&app_state.db, &user, &share_id, params.model_ids).await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_share(
    CurrentUser(user): CurrentUser,
    Path(share_id): Path<String>,
    State(app_state): State<AppState>,
) -> Result<StatusCode, ApplicationError> {
    share_db::delete_share(&app_state.db, &user, &share_id).await?;

    Ok(StatusCode::NO_CONTENT)
}
