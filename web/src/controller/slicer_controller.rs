use axum::{
    Json, Router,
    extract::State,
    response::{IntoResponse, Response},
    routing::post,
};
use axum_login::login_required;
use serde::{Deserialize, Serialize};

use service::AppState;
use service::slice_service::slice_model_for_user;

use crate::{
    error::ApplicationError,
    user::{Backend, CurrentUser},
};

pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/slicer/slice", post(post_slice))
            .route_layer(login_required!(Backend)),
    )
}

/// JSON body for `POST /api/v1/slicer/slice`. Accepts `snake_case` (Rust) and common camelCase
/// aliases for mobile contract alignment. Unknown fields (e.g. a `settings` object from older
/// clients) are ignored; the slicer's own defaults apply until a CLI mapping exists.
#[derive(Debug, Deserialize)]
pub struct SliceRequestBody {
    #[serde(alias = "modelId")]
    pub model_id: i64,
}

/// Success payload for slice orchestration. JSON uses camelCase per `docs/api/v1-mobile-remote.md`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SliceResponseJson {
    pub success: bool,
    pub output_model_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

async fn post_slice(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    Json(body): Json<SliceRequestBody>,
) -> Result<Response, ApplicationError> {
    let result = slice_model_for_user(&state, &user, body.model_id).await?;

    Ok(Json(SliceResponseJson {
        success: true,
        output_model_id: result.output_model_id,
        message: None,
    })
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::{SliceRequestBody, SliceResponseJson};

    // The mobile client posts `{ modelId }` and reads `outputModelId`.
    #[test]
    fn slice_body_accepts_both_spellings_and_response_is_camel_case() {
        for body in [r#"{"modelId":7}"#, r#"{"model_id":7}"#] {
            let parsed: SliceRequestBody = serde_json::from_str(body).unwrap();
            assert_eq!(parsed.model_id, 7);
        }

        let json = serde_json::to_value(SliceResponseJson {
            success: true,
            output_model_id: 9,
            message: None,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"success": true, "outputModelId": 9})
        );
    }
}
