use axum::{
    Json, Router,
    extract::State,
    response::{IntoResponse, Response},
    routing::post,
};
use axum_login::login_required;
use serde::{Deserialize, Serialize};

use service::slice_service::{
    SliceOrchestrationResult, SliceOrchestrationSettings, slice_model_for_user,
};

use crate::{
    error::ApplicationError,
    user::{AuthSession, Backend},
    web_app_state::WebAppState,
};

pub fn router() -> Router<WebAppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/slicer/slice", post(post_slice))
            .route_layer(login_required!(Backend)),
    )
}

/// JSON body for `POST /api/v1/slicer/slice`. Accepts `snake_case` (Rust) and common camelCase
/// aliases for mobile contract alignment.
#[derive(Debug, Deserialize)]
pub struct SliceRequestBody {
    #[serde(alias = "modelId")]
    pub model_id: i64,
    #[serde(default)]
    pub settings: SliceSettingsDto,
}

#[derive(Debug, Default, Deserialize)]
pub struct SliceSettingsDto {
    #[serde(default, alias = "layerHeight")]
    pub layer_height_mm: Option<f64>,
    #[serde(default, alias = "infill")]
    pub infill_percent: Option<u8>,
}

impl From<SliceSettingsDto> for SliceOrchestrationSettings {
    fn from(d: SliceSettingsDto) -> Self {
        Self {
            layer_height_mm: d.layer_height_mm,
            infill_percent: d.infill_percent,
        }
    }
}

/// Success payload for slice orchestration. JSON uses camelCase per `docs/api/v1-mobile-remote.md`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SliceResponseJson {
    pub success: bool,
    pub output_model_id: i64,
    pub output_blob_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

async fn post_slice(
    auth_session: AuthSession,
    State(web_state): State<WebAppState>,
    Json(body): Json<SliceRequestBody>,
) -> Result<Response, ApplicationError> {
    let user = auth_session.user.unwrap().to_user();
    let settings = SliceOrchestrationSettings::from(body.settings);
    let result =
        slice_model_for_user(&web_state.app_state, &user, body.model_id, &settings).await?;

    Ok(Json(slice_response_from_result(result)).into_response())
}

fn slice_response_from_result(result: SliceOrchestrationResult) -> SliceResponseJson {
    SliceResponseJson {
        success: true,
        output_model_id: result.output_model_id,
        output_blob_sha256: result.output_blob_sha256,
        message: None,
    }
}
