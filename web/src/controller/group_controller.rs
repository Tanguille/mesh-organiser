use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use axum_extra::extract::Query;
use axum_login::login_required;
use serde::{Deserialize, Serialize};

use db::{group_db, group_db::GroupFilterOptions, model::blob::FileType};
use service::AppState;

use crate::{
    controller::{ModelIdsParams, share_controller::resolve_share_owner},
    error::ApplicationError,
    query_bounds,
    user::{Backend, CurrentUser},
};

pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/groups", get(get_groups))
            .route("/groups/count", get(get_group_count))
            .route("/groups", post(add_group))
            .route("/groups/detach_models", delete(remove_models_from_group))
            .route("/groups/{group_id}", put(edit_group))
            .route("/groups/{group_id}", delete(delete_group))
            .route("/groups/{group_id}/models", post(add_models_to_group))
            .route_layer(login_required!(Backend))
            .route("/shares/{share_id}/groups", get(get_share_groups)),
    )
}

#[derive(Deserialize)]
pub struct GetGroupParams {
    #[serde(default)]
    pub model_ids: Vec<i64>,
    pub model_ids_str: Option<String>,
    #[serde(default)]
    pub group_ids: Vec<i64>,
    #[serde(default)]
    pub label_ids: Vec<i64>,
    pub order_by: Option<String>,
    pub text_search: Option<String>,
    #[serde(default)]
    pub file_types: Vec<FileType>,
    pub page: u32,
    pub page_size: u32,
    pub include_ungrouped_models: Option<bool>,
}

impl GetGroupParams {
    fn paginated_bounds(&self) -> query_bounds::PaginatedListQueryBounds<'_> {
        query_bounds::PaginatedListQueryBounds {
            model_ids: &self.model_ids,
            group_ids: &self.group_ids,
            label_ids: &self.label_ids,
            text_search: self.text_search.as_deref(),
            order_by: self.order_by.as_deref(),
            page: self.page,
            page_size: self.page_size,
        }
    }
}

pub async fn get_groups(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
    Query(params): Query<GetGroupParams>,
) -> Result<Response, ApplicationError> {
    if let Err(e) = query_bounds::validate_group_list_query_bounds(
        params.paginated_bounds(),
        params.model_ids_str.as_deref(),
    ) {
        return Ok(query_bounds::bad_request(&e));
    }

    let model_ids =
        match query_bounds::optional_comma_separated_model_ids(params.model_ids_str.as_deref()) {
            Ok(model_ids) => model_ids,
            Err(e) => return Ok(query_bounds::bad_request(&e)),
        };

    let groups = group_db::get_groups(
        &app_state.db,
        &user,
        GroupFilterOptions {
            model_ids: if params.model_ids.is_empty() {
                model_ids
            } else {
                Some(params.model_ids)
            },
            group_ids: query_bounds::none_if_empty(params.group_ids),
            label_ids: query_bounds::none_if_empty(params.label_ids),
            order_by: params.order_by.as_deref().map(|order_by| {
                query_bounds::parse_order_by_bounded(order_by, group_db::GroupOrderBy::NameAsc)
            }),
            text_search: params.text_search,
            file_types: query_bounds::none_if_empty(params.file_types),
            page: params.page,
            page_size: params.page_size,
            include_ungrouped_models: params.include_ungrouped_models.unwrap_or(false),
            allow_incomplete_groups: false,
            split_incomplete_groups: false,
        },
    )
    .await?;

    Ok(Json(groups.items).into_response())
}

pub async fn get_share_groups(
    Path(share_id): Path<String>,
    State(app_state): State<AppState>,
    Query(params): Query<GetGroupParams>,
) -> Result<Response, ApplicationError> {
    let (share, user) = resolve_share_owner(&app_state, &share_id).await?;

    if let Err(e) = query_bounds::validate_group_list_query_bounds(
        params.paginated_bounds(),
        params.model_ids_str.as_deref(),
    ) {
        return Ok(query_bounds::bad_request(&e));
    }

    let groups = group_db::get_groups(
        &app_state.db,
        &user,
        GroupFilterOptions {
            model_ids: share.model_ids.into(),
            group_ids: query_bounds::none_if_empty(params.group_ids),
            label_ids: None,
            order_by: params.order_by.as_deref().map(|order_by| {
                query_bounds::parse_order_by_bounded(order_by, group_db::GroupOrderBy::NameAsc)
            }),
            text_search: params.text_search,
            file_types: query_bounds::none_if_empty(params.file_types),
            page: params.page,
            page_size: params.page_size,
            include_ungrouped_models: params.include_ungrouped_models.unwrap_or(true),
            allow_incomplete_groups: true,
            split_incomplete_groups: false,
        },
    )
    .await?;

    Ok(Json(groups.items).into_response())
}

#[derive(Deserialize)]
pub struct GetGroupCountParams {
    pub include_ungrouped_models: Option<bool>,
}

#[derive(Serialize)]
pub struct GetGroupCountResponse {
    pub count: usize,
}

pub async fn get_group_count(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
    Query(params): Query<GetGroupCountParams>,
) -> Result<Json<GetGroupCountResponse>, ApplicationError> {
    let count = group_db::get_group_count(
        &app_state.db,
        &user,
        params.include_ungrouped_models.unwrap_or(false),
    )
    .await?;

    Ok(Json(GetGroupCountResponse { count }))
}

#[derive(Deserialize)]
#[allow(clippy::struct_field_names)] // field names match API
pub struct PutGroupParams {
    pub group_name: String,
    pub group_timestamp: Option<String>,
    pub group_global_id: Option<String>,
}

pub async fn edit_group(
    CurrentUser(user): CurrentUser,
    Path(group_id): Path<i64>,
    State(app_state): State<AppState>,
    Json(params): Json<PutGroupParams>,
) -> Result<StatusCode, ApplicationError> {
    group_db::edit_group(
        &app_state.db,
        &user,
        group_id,
        &params.group_name,
        params.group_timestamp.as_deref(),
        params.group_global_id.as_deref(),
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_group(
    CurrentUser(user): CurrentUser,
    Path(group_id): Path<i64>,
    State(app_state): State<AppState>,
) -> Result<StatusCode, ApplicationError> {
    group_db::delete_group(&app_state.db, &user, group_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_models_from_group(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
    Json(params): Json<ModelIdsParams>,
) -> Result<StatusCode, ApplicationError> {
    group_db::set_group_id_on_models(&app_state.db, &user, None, params.model_ids, None).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct PostGroupParams {
    pub group_name: String,
}

pub async fn add_group(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
    Json(params): Json<PostGroupParams>,
) -> Result<Response, ApplicationError> {
    let group_meta =
        group_db::add_empty_group(&app_state.db, &user, &params.group_name, None).await?;

    Ok(Json(group_meta).into_response())
}

pub async fn add_models_to_group(
    CurrentUser(user): CurrentUser,
    Path(group_id): Path<i64>,
    State(app_state): State<AppState>,
    Json(params): Json<ModelIdsParams>,
) -> Result<StatusCode, ApplicationError> {
    group_db::set_group_id_on_models(&app_state.db, &user, Some(group_id), params.model_ids, None)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
