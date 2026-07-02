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
    user::{AuthSession, Backend},
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
        ApplicationError, AuthSession, IntoResponse, Json, Path, Response, State, StatusCode,
        WebAppState, model_db, threemf_service,
    };

    pub async fn get_threemf_metadata(
        auth_session: AuthSession,
        Path(model_id): Path<i64>,
        State(app_state): State<WebAppState>,
    ) -> Result<Response, ApplicationError> {
        let user = auth_session.user.unwrap().to_user();

        let model =
            model_db::get_models_via_ids(&app_state.app_state.db, &user, vec![model_id]).await?;

        if model.is_empty() {
            return Ok((StatusCode::NOT_FOUND, "Model not found").into_response());
        }

        let threemf_metadata =
            threemf_service::extract_metadata(&model[0], &app_state.app_state).await?;

        Ok(Json(threemf_metadata).into_response())
    }
}

mod post {
    use service::thumbnail_service;

    use crate::web_import_state::WebImportStateEmitter;

    use super::{
        ApplicationError, AuthSession, IntoResponse, Json, Path, Response, State, StatusCode,
        WebAppState, model_db, threemf_service,
    };

    pub async fn extract_threemf_models(
        auth_session: AuthSession,
        Path(model_id): Path<i64>,
        State(app_state): State<WebAppState>,
    ) -> Result<Response, ApplicationError> {
        let user = auth_session.user.unwrap().to_user();

        let model =
            model_db::get_models_via_ids(&app_state.app_state.db, &user, vec![model_id]).await?;

        if model.is_empty() {
            return Ok((StatusCode::NOT_FOUND, "Model not found").into_response());
        }

        let mut import_state =
            threemf_service::extract_models(&model[0], &user, &app_state.app_state).await?;

        import_state.set_emitter(Box::new(WebImportStateEmitter {}));

        let model_ids = import_state.all_model_ids();

        thumbnail_service::generate_thumbnails_for_model_ids(
            &app_state.app_state,
            &user,
            model_ids,
            &mut import_state,
        )
        .await?;

        let group_meta = threemf_service::group_meta_from_import(&import_state)?;

        Ok(Json(group_meta).into_response())
    }
}
