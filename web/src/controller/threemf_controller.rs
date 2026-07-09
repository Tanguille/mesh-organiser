use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_login::login_required;

use db::model_db;
use service::threemf_service;

use crate::{
    error::ApplicationError,
    user::{Backend, CurrentUser},
    web_app_state::WebAppState,
};

pub fn router() -> Router<WebAppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route(
                "/models/{model_id}/3mf_metadata",
                get(get::get_threemf_metadata),
            )
            .route(
                "/models/{model_id}/3mf_extract",
                post(post::extract_threemf_models),
            )
            .route_layer(login_required!(Backend)),
    )
}

mod get {
    use super::{
        ApplicationError, CurrentUser, IntoResponse, Json, Path, Response, State, StatusCode,
        WebAppState, model_db, threemf_service,
    };

    pub async fn get_threemf_metadata(
        CurrentUser(user): CurrentUser,
        Path(model_id): Path<i64>,
        State(app_state): State<WebAppState>,
    ) -> Result<Response, ApplicationError> {
        let Some(model) =
            model_db::get_model_via_id(&app_state.app_state.db, &user, model_id).await?
        else {
            return Ok((StatusCode::NOT_FOUND, "Model not found").into_response());
        };

        let threemf_metadata =
            threemf_service::extract_metadata(&model, &app_state.app_state).await?;

        Ok(Json(threemf_metadata).into_response())
    }
}

mod post {
    use crate::web_import_state::WebImportStateEmitter;

    use super::{
        ApplicationError, CurrentUser, IntoResponse, Json, Path, Response, State, StatusCode,
        WebAppState, model_db, threemf_service,
    };

    pub async fn extract_threemf_models(
        CurrentUser(user): CurrentUser,
        Path(model_id): Path<i64>,
        State(app_state): State<WebAppState>,
    ) -> Result<Response, ApplicationError> {
        let Some(model) =
            model_db::get_model_via_id(&app_state.app_state.db, &user, model_id).await?
        else {
            return Ok((StatusCode::NOT_FOUND, "Model not found").into_response());
        };

        let group_meta = threemf_service::extract_models_with_thumbnails(
            &model,
            &user,
            &app_state.app_state,
            Some(Box::new(WebImportStateEmitter {})),
        )
        .await?;

        Ok(Json(group_meta).into_response())
    }
}
