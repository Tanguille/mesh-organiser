use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use axum_extra::extract::Query;
use serde::Deserialize;

use db::group_db::{self, GroupFilterOptions};
use service::AppState;

use crate::{controller::ModelIdsParams, error::ApplicationError, query_bounds, user::CurrentUser};

pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/groups", get(get_groups))
            .route("/groups/count", get(get_group_count))
            .route("/groups", post(add_group))
            .route("/groups/trash", get(get_trashed_groups))
            .route("/groups/restore", post(restore_groups))
            .route("/groups/detach_models", delete(remove_models_from_group))
            .route("/groups/{group_id}", axum::routing::put(edit_group))
            .route("/groups/{group_id}", delete(delete_group))
            .route("/groups/{group_id}/models", post(add_models_to_group))
            .route_layer(axum_login::login_required!(crate::user::Backend))
            .route("/shares/{share_id}/groups", get(get_share_groups)),
    )
}

pub async fn get_trashed_groups(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
) -> Result<Json<Vec<group_db::DeletedGroup>>, ApplicationError> {
    let retention_days = app_state.get_configuration().trash_retention_days;
    Ok(Json(
        group_db::get_trashed_groups(&app_state.db, &user, retention_days).await?,
    ))
}

#[derive(Deserialize)]
pub struct RestoreGroupsParams {
    pub groups: Vec<group_db::GroupRestoreRequest>,
}

pub async fn restore_groups(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
    Json(params): Json<RestoreGroupsParams>,
) -> Result<StatusCode, ApplicationError> {
    let retention_days = app_state.get_configuration().trash_retention_days;
    group_db::restore_groups(&app_state.db, &user, &params.groups, retention_days).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct GetGroupParams {
    pub model_ids_str: Option<String>,
    #[serde(default)]
    pub group_ids: Vec<i64>,
    #[serde(default)]
    pub label_ids: Vec<i64>,
    pub order_by: Option<String>,
    pub text_search: Option<String>,
    #[serde(default)]
    pub file_types: Vec<db::model::blob::FileType>,
    pub page: u32,
    pub page_size: u32,
    pub include_ungrouped_models: Option<bool>,
}

impl GetGroupParams {
    /// Borrows the list-query fields that `query_bounds` validates, so the
    /// bounds check runs before any allocation from request data.
    fn paginated_bounds(&self) -> query_bounds::PaginatedListQueryBounds<'_> {
        query_bounds::PaginatedListQueryBounds {
            model_ids: &[], // model_ids_str is bounded separately before parsing
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
    if let Err(e) = query_bounds::validate_list_query_bounds(
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
            model_ids,
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
    let scope =
        crate::controller::share_controller::ShareScope::resolve(&app_state, &share_id).await?;

    if let Err(e) = query_bounds::validate_list_query_bounds(
        params.paginated_bounds(),
        params.model_ids_str.as_deref(),
    ) {
        return Ok(query_bounds::bad_request(&e));
    }

    // Share membership replaces the request's model filter, including for an empty share.
    let groups = group_db::get_groups(
        &app_state.db,
        &scope.owner,
        GroupFilterOptions {
            model_ids: Some(scope.share.model_ids),
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

#[derive(serde::Serialize)]
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

#[cfg(test)]
#[path = "group_recovery_tests.rs"]
mod group_recovery_tests;
