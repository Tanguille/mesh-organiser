use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use axum_extra::extract::Query;
use axum_login::login_required;
use serde::Deserialize;

use db::{label_db, label_keyword_db};

use crate::{
    error::ApplicationError,
    user::{Backend, CurrentUser},
    web_app_state::WebAppState,
};

pub fn router() -> Router<WebAppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/labels", get(get_labels))
            .route("/labels", post(add_label))
            .route("/labels/{label_id}", put(edit_label))
            .route("/labels/{label_id}", delete(delete_label))
            .route("/labels/{label_id}/models", post(set_label_on_models))
            .route(
                "/labels/{label_id}/models",
                delete(remove_label_from_models),
            )
            .route("/labels/{label_id}/childs", put(set_childs_on_label))
            .route("/labels/{label_id}/keywords", get(get_keywords_for_label))
            .route("/labels/{label_id}/keywords", put(set_keywords_on_label))
            .route("/models/{model_id}/labels", put(set_labels_on_model))
            .route_layer(login_required!(Backend)),
    )
}

#[derive(Deserialize)]
pub struct GetLabelsParams {
    pub include_ungrouped_models: Option<bool>,
}

pub async fn get_labels(
    CurrentUser(user): CurrentUser,
    State(app_state): State<WebAppState>,
    Query(params): Query<GetLabelsParams>,
) -> Result<Response, ApplicationError> {
    let labels = label_db::get_labels(
        &app_state.app_state.db,
        &user,
        params.include_ungrouped_models.unwrap_or(false),
    )
    .await?;

    Ok(Json(labels).into_response())
}

pub async fn get_keywords_for_label(
    CurrentUser(user): CurrentUser,
    Path(label_id): Path<i64>,
    State(app_state): State<WebAppState>,
) -> Result<Response, ApplicationError> {
    let keywords =
        label_keyword_db::get_keywords_for_label(&app_state.app_state.db, &user, label_id).await?;

    Ok(Json(keywords).into_response())
}

#[derive(Deserialize)]
pub struct PostLabelParams {
    pub label_name: String,
    pub label_color: i64,
}

pub async fn add_label(
    CurrentUser(user): CurrentUser,
    State(app_state): State<WebAppState>,
    Json(params): Json<PostLabelParams>,
) -> Result<Response, ApplicationError> {
    let label_meta = label_db::add_label(
        &app_state.app_state.db,
        &user,
        &params.label_name,
        params.label_color,
        None,
    )
    .await?;

    Ok(Json(label_meta).into_response())
}

pub async fn set_label_on_models(
    CurrentUser(user): CurrentUser,
    Path(label_id): Path<i64>,
    State(app_state): State<WebAppState>,
    Json(params): Json<crate::controller::ModelIdsParams>,
) -> Result<StatusCode, ApplicationError> {
    label_db::remove_labels_from_models(
        &app_state.app_state.db,
        &user,
        &[label_id],
        &params.model_ids,
        None,
    )
    .await?;
    label_db::add_labels_on_models(
        &app_state.app_state.db,
        &user,
        &[label_id],
        &params.model_ids,
        None,
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[allow(clippy::struct_field_names)] // field names match API
pub struct PutLabelParams {
    pub label_name: String,
    pub label_color: i64,
    pub label_timestamp: Option<String>,
    pub label_global_id: Option<String>,
}

pub async fn edit_label(
    CurrentUser(user): CurrentUser,
    Path(label_id): Path<i64>,
    State(app_state): State<WebAppState>,
    Json(params): Json<PutLabelParams>,
) -> Result<StatusCode, ApplicationError> {
    label_db::edit_label(
        &app_state.app_state.db,
        &user,
        label_id,
        &params.label_name,
        params.label_color,
        params.label_timestamp.as_deref(),
        params.label_global_id.as_deref(),
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct SetLabelsOnModelParams {
    pub label_ids: Vec<i64>,
}

pub async fn set_labels_on_model(
    CurrentUser(user): CurrentUser,
    Path(model_id): Path<i64>,
    State(app_state): State<WebAppState>,
    Json(params): Json<SetLabelsOnModelParams>,
) -> Result<StatusCode, ApplicationError> {
    label_db::remove_all_labels_from_models(&app_state.app_state.db, &user, &[model_id], None)
        .await?;
    label_db::add_labels_on_models(
        &app_state.app_state.db,
        &user,
        &params.label_ids,
        &[model_id],
        None,
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct SetChildsOnLabelParams {
    pub child_label_ids: Vec<i64>,
}

pub async fn set_childs_on_label(
    CurrentUser(user): CurrentUser,
    Path(parent_label_id): Path<i64>,
    State(app_state): State<WebAppState>,
    Json(params): Json<SetChildsOnLabelParams>,
) -> Result<StatusCode, ApplicationError> {
    label_db::remove_all_childs_from_label(&app_state.app_state.db, &user, parent_label_id, None)
        .await?;

    if !params.child_label_ids.is_empty() {
        label_db::add_childs_to_label(
            &app_state.app_state.db,
            &user,
            parent_label_id,
            params.child_label_ids,
            None,
        )
        .await?;
    }

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct SetKeywordsOnLabelParams {
    pub keywords: Vec<String>,
}

pub async fn set_keywords_on_label(
    CurrentUser(user): CurrentUser,
    Path(label_id): Path<i64>,
    State(app_state): State<WebAppState>,
    Json(params): Json<SetKeywordsOnLabelParams>,
) -> Result<StatusCode, ApplicationError> {
    label_keyword_db::set_keywords_for_label(
        &app_state.app_state.db,
        &user,
        label_id,
        params.keywords,
        None,
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_label(
    CurrentUser(user): CurrentUser,
    Path(label_id): Path<i64>,
    State(app_state): State<WebAppState>,
) -> Result<StatusCode, ApplicationError> {
    label_db::delete_label(&app_state.app_state.db, &user, label_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_label_from_models(
    CurrentUser(user): CurrentUser,
    Path(label_id): Path<i64>,
    State(app_state): State<WebAppState>,
    Json(params): Json<crate::controller::ModelIdsParams>,
) -> Result<StatusCode, ApplicationError> {
    label_db::remove_labels_from_models(
        &app_state.app_state.db,
        &user,
        &[label_id],
        &params.model_ids,
        None,
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}
