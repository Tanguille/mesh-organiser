use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::Query;
use axum_login::login_required;
use serde::Deserialize;
use tokio::{fs::File, io::BufReader};
use tokio_util::io::ReaderStream;

use db::{
    model::{blob::Blob, user::User},
    model_db, user_db,
};
use service::{cleanse_evil_from_name, convert_zip_to_extension, export_service};

use crate::{
    error::ApplicationError,
    user::{Backend, CurrentUser},
    web_app_state::WebAppState,
};

pub fn router() -> Router<WebAppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/models/{model_id}/bytes", get(get::get_model_bytes))
            .route("/blobs/{sha256}/bytes", get(get::get_blob_bytes))
            .route("/blobs/download", post(post::create_blobs_zip_download))
            .route_layer(login_required!(Backend))
            .route("/blobs/{sha256}/thumb", get(get::get_blob_thumb))
            .route("/blobs/{sha256}/download", get(get::download_model))
            .route(
                "/blobs/download/{zip_dir}",
                get(get::get_blobs_zip_download),
            ),
    )
}

mod get {
    use crate::path_safety::{resolve_path_under_base, resolve_path_under_temp};

    use super::{
        ApplicationError, Blob, Body, BufReader, CurrentUser, Deserialize, File, IntoResponse,
        Path, Query, ReaderStream, Response, State, StatusCode, User, WebAppState,
        cleanse_evil_from_name, convert_zip_to_extension, export_service, model_db, user_db,
    };

    #[derive(Deserialize)]
    pub struct DownloadModelParams {
        pub user_id: Option<i64>,
        pub user_hash: Option<String>,
        pub share_id: Option<String>,
    }

    async fn extract_user_via_id_and_hash(
        app_state: &WebAppState,
        user_id: i64,
        user_hash: &str,
    ) -> Option<User> {
        let Ok(Some(user)) = user_db::get_user_by_id(&app_state.app_state.db, user_id).await else {
            return None;
        };

        if user.sync_url.as_deref() != Some(user_hash) {
            return None;
        }

        Some(user)
    }

    async fn extract_user_via_share_id(app_state: &WebAppState, share_id: &str) -> Option<User> {
        crate::controller::share_controller::resolve_share_owner(app_state, share_id)
            .await
            .ok()
            .map(|(_share, user)| user)
    }

    pub async fn download_model(
        Path(blob_sha256): Path<String>,
        State(app_state): State<WebAppState>,
        Query(params): Query<DownloadModelParams>,
    ) -> Response {
        let user = match params {
            DownloadModelParams {
                user_id: Some(user_id),
                user_hash: Some(user_hash),
                share_id: None,
            } => match extract_user_via_id_and_hash(&app_state, user_id, &user_hash).await {
                Some(u) => u,
                None => return StatusCode::NOT_FOUND.into_response(),
            },
            DownloadModelParams {
                user_id: None,
                user_hash: None,
                share_id: Some(share_id),
            } => match extract_user_via_share_id(&app_state, &share_id).await {
                Some(u) => u,
                None => return StatusCode::NOT_FOUND.into_response(),
            },
            _ => return StatusCode::NOT_FOUND.into_response(),
        };

        let Ok(Some(model_id)) =
            model_db::get_model_id_via_sha256(&app_state.app_state.db, &user, &blob_sha256).await
        else {
            return StatusCode::NOT_FOUND.into_response();
        };

        let Ok(Some(model)) =
            model_db::get_model_via_id(&app_state.app_state.db, &user, model_id).await
        else {
            return StatusCode::NOT_FOUND.into_response();
        };

        if model.blob.sha256 != blob_sha256 {
            return StatusCode::NOT_FOUND.into_response();
        }

        let filename = format!(
            "{}.{}",
            cleanse_evil_from_name(&model.name).trim(),
            convert_zip_to_extension(&model.blob.filetype)
        )
        .to_ascii_lowercase();
        let mut response = get_blob_bytes_inner(&model.blob, &app_state).await;

        response.headers_mut().insert(
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\"")
                .parse()
                .unwrap(),
        );

        response
    }

    pub async fn get_model_bytes(
        CurrentUser(user): CurrentUser,
        Path(model_id): Path<i64>,
        State(app_state): State<WebAppState>,
    ) -> Response {
        let Ok(Some(model)) =
            model_db::get_model_via_id(&app_state.app_state.db, &user, model_id).await
        else {
            return StatusCode::NOT_FOUND.into_response();
        };

        get_blob_bytes_inner(&model.blob, &app_state).await
    }

    pub async fn get_blob_bytes(
        CurrentUser(user): CurrentUser,
        Path(sha256): Path<String>,
        State(app_state): State<WebAppState>,
    ) -> Response {
        // Verify that the user has access to a model with this blob
        let Ok(Some(_model_id)) =
            model_db::get_model_id_via_sha256(&app_state.app_state.db, &user, &sha256).await
        else {
            return StatusCode::NOT_FOUND.into_response();
        };

        let Ok(Some(blob)) =
            db::blob_db::get_blob_via_sha256(&app_state.app_state.db, &sha256).await
        else {
            return StatusCode::NOT_FOUND.into_response();
        };

        get_blob_bytes_inner(&blob, &app_state)
            .await
            .into_response()
    }

    pub async fn get_blob_thumb(
        Path(sha256): Path<String>,
        State(app_state): State<WebAppState>,
    ) -> Result<Response, ApplicationError> {
        // Validate that the sha256 parameter is a well-formed SHA-256 hex string.
        // Ensures a single, safe filename component (no separators).
        if sha256.len() != 64 || !sha256.chars().all(|c: char| c.is_ascii_hexdigit()) {
            return Ok(StatusCode::BAD_REQUEST.into_response());
        }

        let base_dir = app_state.get_image_dir();
        let src_file_path = match resolve_path_under_base(&base_dir, &format!("{sha256}.png")).await
        {
            Ok(path) => path,
            Err(e) => return e.respond(),
        };

        let Ok(file) = File::open(src_file_path).await else {
            return Ok(StatusCode::NOT_FOUND.into_response());
        };

        let buffered_reader = BufReader::new(file);
        let stream = ReaderStream::new(buffered_reader);

        Ok(Body::from_stream(stream).into_response())
    }

    async fn get_blob_bytes_inner(blob: &Blob, app_state: &WebAppState) -> Response {
        // The plain-file vs first-zip-entry convention lives in the service
        // layer; any open/read failure keeps the previous 500 semantics.
        let Ok(reader) = export_service::open_blob_content_reader(blob, &app_state.app_state).await
        else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };

        Body::from_stream(ReaderStream::new(reader)).into_response()
    }

    pub async fn get_blobs_zip_download(
        Path(zip_dir): Path<String>,
    ) -> Result<Response, ApplicationError> {
        if !zip_dir.starts_with("meshorganiser_") {
            return Ok(StatusCode::BAD_REQUEST.into_response());
        }

        let path = match resolve_path_under_temp(&zip_dir).await {
            Ok(path) => path,
            Err(e) => return e.respond(),
        };

        let mut list_dir = tokio::fs::read_dir(&path).await?;
        let next = list_dir.next_entry().await?;

        let Some(zip_file) = next else {
            return Ok(StatusCode::NOT_FOUND.into_response());
        };

        let file_name = zip_file.file_name();
        let name_lossy = file_name.to_string_lossy();
        if !name_lossy.ends_with(".zip") {
            return Ok(StatusCode::NOT_FOUND.into_response());
        }

        let path = zip_file.path();

        if !path.exists() {
            return Ok(StatusCode::NOT_FOUND.into_response());
        }

        let file = File::open(path).await?;
        let buffered_reader = BufReader::new(file);
        let stream = ReaderStream::new(buffered_reader);

        let mut response = Body::from_stream(stream).into_response();

        response.headers_mut().insert(
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name_lossy}\"")
                .parse()
                .unwrap(),
        );

        Ok(response)
    }
}

mod post {
    use super::{
        ApplicationError, CurrentUser, IntoResponse, Json, Response, State, StatusCode,
        WebAppState, export_service, model_db,
    };

    pub async fn create_blobs_zip_download(
        CurrentUser(user): CurrentUser,
        State(app_state): State<WebAppState>,
        Json(blob_sha256s): Json<Vec<String>>,
    ) -> Result<Response, ApplicationError> {
        let Ok(model_ids) =
            model_db::get_model_ids_via_sha256s(&app_state.app_state.db, &user, &blob_sha256s)
                .await
        else {
            return Ok(StatusCode::NOT_FOUND.into_response());
        };

        let Ok(models) =
            model_db::get_models_via_ids(&app_state.app_state.db, &user, model_ids).await
        else {
            return Ok(StatusCode::NOT_FOUND.into_response());
        };

        if models.len() != blob_sha256s.len() {
            return Ok(StatusCode::NOT_FOUND.into_response());
        }

        let path = export_service::export_zip_to_temp_folder(models, &app_state.app_state).await?;

        Ok(Json(
            path.temp_dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string(),
        )
        .into_response())
    }
}
