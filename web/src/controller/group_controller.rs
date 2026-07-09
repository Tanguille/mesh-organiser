use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use axum_login::login_required;
use serde::{Deserialize, Serialize};

use db::{group_db, group_db::GroupFilterOptions};

use crate::{
    error::ApplicationError,
    user::{Backend, CurrentUser},
    web_app_state::WebAppState,
};

pub fn router() -> Router<WebAppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/groups", get(get::get_groups))
            .route("/groups/count", get(get::get_group_count))
            .route("/groups", post(post::add_group))
            .route(
                "/groups/detach_models",
                delete(delete::remove_models_from_group),
            )
            .route("/groups/{group_id}", put(put::edit_group))
            .route("/groups/{group_id}", delete(delete::delete_group))
            .route("/groups/{group_id}/models", post(post::add_models_to_group))
            .route_layer(login_required!(Backend))
            .route("/shares/{share_id}/groups", get(get::get_share_groups)),
    )
}

mod get {
    use axum_extra::extract::Query;

    use crate::{controller::share_controller::resolve_share_owner, query_bounds};

    use super::{
        ApplicationError, CurrentUser, Deserialize, GroupFilterOptions, IntoResponse, Json, Path,
        Response, Serialize, State, WebAppState, group_db,
    };

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
        State(app_state): State<WebAppState>,
        Query(params): Query<GetGroupParams>,
    ) -> Result<Response, ApplicationError> {
        if let Err(e) = query_bounds::validate_group_list_query_bounds(
            params.paginated_bounds(),
            params.model_ids_str.as_deref(),
        ) {
            return Ok(query_bounds::bad_request(&e));
        }

        let model_ids =
            match query_bounds::optional_comma_separated_model_ids(params.model_ids_str.as_deref())
            {
                Ok(v) => v,
                Err(e) => return Ok(query_bounds::bad_request(&e)),
            };

        let groups = group_db::get_groups(
            &app_state.app_state.db,
            &user,
            GroupFilterOptions {
                model_ids: if params.model_ids.is_empty() {
                    model_ids
                } else {
                    Some(params.model_ids)
                },
                group_ids: query_bounds::none_if_empty(params.group_ids),
                label_ids: query_bounds::none_if_empty(params.label_ids),
                order_by: params
                    .order_by
                    .as_deref()
                    .map(query_bounds::parse_group_order_by_bounded),
                text_search: params.text_search,
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
        State(app_state): State<WebAppState>,
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
            &app_state.app_state.db,
            &user,
            GroupFilterOptions {
                model_ids: share.model_ids.into(),
                group_ids: query_bounds::none_if_empty(params.group_ids),
                label_ids: None,
                order_by: params
                    .order_by
                    .as_deref()
                    .map(query_bounds::parse_group_order_by_bounded),
                text_search: params.text_search,
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
        State(app_state): State<WebAppState>,
        Query(params): Query<GetGroupCountParams>,
    ) -> Result<Response, ApplicationError> {
        let count = group_db::get_group_count(
            &app_state.app_state.db,
            &user,
            params.include_ungrouped_models.unwrap_or(false),
        )
        .await?;

        Ok(Json(GetGroupCountResponse { count }).into_response())
    }
}

mod put {
    use super::{
        ApplicationError, CurrentUser, Deserialize, IntoResponse, Json, Path, Response, State,
        StatusCode, WebAppState, group_db,
    };

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
        State(app_state): State<WebAppState>,
        Json(params): Json<PutGroupParams>,
    ) -> Result<Response, ApplicationError> {
        group_db::edit_group(
            &app_state.app_state.db,
            &user,
            group_id,
            &params.group_name,
            params.group_timestamp.as_deref(),
            params.group_global_id.as_deref(),
        )
        .await?;

        Ok(StatusCode::NO_CONTENT.into_response())
    }
}

mod delete {
    use super::{
        ApplicationError, CurrentUser, Deserialize, IntoResponse, Json, Path, Response, State,
        StatusCode, WebAppState, group_db,
    };

    pub async fn delete_group(
        CurrentUser(user): CurrentUser,
        Path(group_id): Path<i64>,
        State(app_state): State<WebAppState>,
    ) -> Result<Response, ApplicationError> {
        group_db::delete_group(&app_state.app_state.db, &user, group_id).await?;

        Ok(StatusCode::NO_CONTENT.into_response())
    }

    #[derive(Deserialize)]
    pub struct RemoveModelsFromGroupParams {
        pub model_ids: Vec<i64>,
    }

    pub async fn remove_models_from_group(
        CurrentUser(user): CurrentUser,
        State(app_state): State<WebAppState>,
        Json(params): Json<RemoveModelsFromGroupParams>,
    ) -> Result<Response, ApplicationError> {
        group_db::set_group_id_on_models(
            &app_state.app_state.db,
            &user,
            None,
            params.model_ids,
            None,
        )
        .await?;

        Ok(StatusCode::NO_CONTENT.into_response())
    }
}

mod post {
    use super::{
        ApplicationError, CurrentUser, Deserialize, IntoResponse, Json, Path, Response, State,
        StatusCode, WebAppState, group_db,
    };

    #[derive(Deserialize)]
    pub struct PostGroupParams {
        pub group_name: String,
    }

    pub async fn add_group(
        CurrentUser(user): CurrentUser,
        State(app_state): State<WebAppState>,
        Json(params): Json<PostGroupParams>,
    ) -> Result<Response, ApplicationError> {
        let group_meta =
            group_db::add_empty_group(&app_state.app_state.db, &user, &params.group_name, None)
                .await?;

        Ok(Json(group_meta).into_response())
    }

    #[derive(Deserialize)]
    pub struct AddModelsToGroupParams {
        pub model_ids: Vec<i64>,
    }

    pub async fn add_models_to_group(
        CurrentUser(user): CurrentUser,
        Path(group_id): Path<i64>,
        State(app_state): State<WebAppState>,
        Json(params): Json<AddModelsToGroupParams>,
    ) -> Result<Response, ApplicationError> {
        group_db::set_group_id_on_models(
            &app_state.app_state.db,
            &user,
            Some(group_id),
            params.model_ids,
            None,
        )
        .await?;

        Ok(StatusCode::NO_CONTENT.into_response())
    }
}
