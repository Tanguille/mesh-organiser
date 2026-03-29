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

/// JSON body for `POST /api/v1/slicer/slice`. Accepts snake_case (Rust) and common camelCase
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

/// Success payload for slice orchestration. Aligns with `SliceResponse` in `docs/api/v1-mobile-remote.md`
/// while exposing `output_model_id` / `output_blob_sha256` from [`SliceOrchestrationResult`].
#[derive(Debug, Serialize)]
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use axum_login::AuthManagerLayerBuilder;
    use serde_json::json;
    use service::{AppState, Configuration};
    use tempfile::{TempDir, tempdir};
    use time::Duration;
    use tower::ServiceExt;
    use tower_sessions::{Expiry, SessionManagerLayer, cookie::Key};
    use tower_sessions_sqlx_store::SqliteStore;

    use crate::user::Backend;
    use crate::web_app_state::WebAppState;

    use super::router;

    async fn slicer_test_router(dir: &TempDir) -> Router<WebAppState> {
        let data_path = dir.path().join("data");
        std::fs::create_dir_all(&data_path).expect("create data path");
        let db_path = data_path.join("db.sqlite");
        let backup_dir = data_path.join("backup");
        std::fs::create_dir_all(&backup_dir).expect("create backup dir");

        let db = db::db_context::setup_db(&db_path, &backup_dir).await;
        let db_clone = db.clone();
        let config = Configuration {
            data_path: data_path.to_string_lossy().to_string(),
            ..Default::default()
        };

        let app_state = AppState {
            db: Arc::new(db),
            configuration: Mutex::new(config),
            import_mutex: Arc::new(tokio::sync::Mutex::new(())),
            app_data_path: data_path.to_string_lossy().to_string(),
        };

        let web_state = WebAppState {
            app_state,
            port: 3000,
        };

        let session_store = SqliteStore::new(db_clone);
        session_store.migrate().await.expect("session migrate");

        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(Duration::days(7)))
            .with_signed(Key::generate());

        let backend = Backend::new(web_state.app_state.db.clone());
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        Router::new()
            .merge(router())
            .with_state(web_state)
            .layer(auth_layer)
    }

    #[tokio::test]
    async fn post_slice_without_session_is_unauthorized() {
        let dir = tempdir().expect("tempdir");
        let app = slicer_test_router(&dir).await;
        let body = json!({
            "model_id": 1,
            "settings": {}
        })
        .to_string();

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/slicer/slice")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .expect("request");

        let response = app.oneshot(req).await.expect("response");
        let status = response.status();
        assert!(
            status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
            "expected 401 or 403 for unauthenticated slice, got {status}"
        );
    }
}
