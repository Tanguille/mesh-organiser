use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use axum_login::login_required;
use serde::Deserialize;

use db::{model::resource::ResourceFlags, resource_db};
use service::{AppState, resource_service};

use crate::{
    error::ApplicationError,
    user::{Backend, CurrentUser},
};

/// Routes for the resource endpoints; `login_required!` guards only those registered before it.
pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/resources", get(get_resources))
            .route("/resources", post(add_resource))
            .route("/resources/{resource_id}", put(edit_resource))
            .route("/resources/{resource_id}", delete(delete_resource))
            .route(
                "/resources/{resource_id}/groups",
                get(get_groups_for_resource),
            )
            .route("/groups/{group_id}/resource", put(set_resource_on_group))
            .route_layer(login_required!(Backend)),
    )
}

pub async fn get_resources(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
) -> Result<Response, ApplicationError> {
    let resources = resource_db::get_resources(&app_state.db, &user).await?;

    Ok(Json(resources).into_response())
}

pub async fn get_groups_for_resource(
    CurrentUser(user): CurrentUser,
    Path(resource_id): Path<i64>,
    State(app_state): State<AppState>,
) -> Result<Response, ApplicationError> {
    let groups = resource_db::get_groups_for_resource(&app_state.db, &user, resource_id).await?;

    Ok(Json(groups).into_response())
}

#[derive(Deserialize)]
pub struct PostResourceParams {
    pub resource_name: String,
}

pub async fn add_resource(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
    Json(params): Json<PostResourceParams>,
) -> Result<Response, ApplicationError> {
    let resource_meta =
        resource_db::add_resource(&app_state.db, &user, &params.resource_name, None).await?;

    Ok(Json(resource_meta).into_response())
}

#[derive(Deserialize)]
#[allow(clippy::struct_field_names)] // field names match API
pub struct PutResourceParams {
    pub resource_name: String,
    pub resource_flags: ResourceFlags,
    pub resource_timestamp: Option<String>,
    pub resource_global_id: Option<String>,
}

pub async fn edit_resource(
    CurrentUser(user): CurrentUser,
    Path(resource_id): Path<i64>,
    State(app_state): State<AppState>,
    Json(params): Json<PutResourceParams>,
) -> Result<StatusCode, ApplicationError> {
    resource_db::edit_resource(
        &app_state.db,
        &user,
        resource_id,
        &params.resource_name,
        params.resource_flags,
        params.resource_timestamp.as_deref(),
        params.resource_global_id.as_deref(),
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct SetResourceOnGroupParams {
    pub resource_id: Option<i64>,
}

pub async fn set_resource_on_group(
    CurrentUser(user): CurrentUser,
    Path(group_id): Path<i64>,
    State(app_state): State<AppState>,
    Json(params): Json<SetResourceOnGroupParams>,
) -> Result<StatusCode, ApplicationError> {
    resource_db::set_resource_on_group(&app_state.db, &user, params.resource_id, group_id, None)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_resource(
    CurrentUser(user): CurrentUser,
    Path(resource_id): Path<i64>,
    State(app_state): State<AppState>,
) -> Result<StatusCode, ApplicationError> {
    resource_service::delete_resource(resource_id, &user, &app_state).await?;

    Ok(StatusCode::NO_CONTENT)
}
