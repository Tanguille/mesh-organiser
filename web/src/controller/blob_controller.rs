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
use tokio::fs::File;
use tokio_util::io::ReaderStream;

use db::{
    model::{blob::Blob, user::User},
    model_db, user_db,
};
use service::{AppState, cleanse_evil_from_name, convert_zip_to_extension, export_service};

use crate::{
    controller::share_controller::ShareScope,
    error::ApplicationError,
    path_safety::resolve_path_under_base,
    user::{Backend, CurrentUser},
};

/// `ReaderStream::new` reads 4 KiB at a time; bigger chunks mean far fewer
/// blocking-pool round trips per file.
const STREAM_CHUNK_SIZE: usize = 64 * 1024;

pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/blobs/{sha256}/bytes", get(get_blob_bytes))
            .route("/blobs/download", post(create_blobs_zip_download))
            .route_layer(login_required!(Backend))
            .route("/blobs/{sha256}/thumb", get(get_blob_thumb))
            .route("/blobs/{sha256}/download", get(download_model))
            .route("/blobs/download/{zip_dir}", get(get_blobs_zip_download)),
    )
}

#[derive(Deserialize)]
pub struct DownloadModelParams {
    pub user_id: Option<i64>,
    pub user_hash: Option<String>,
    pub share_id: Option<String>,
}

async fn extract_user_via_id_and_hash(
    app_state: &AppState,
    user_id: i64,
    user_hash: &str,
) -> Option<User> {
    let Ok(Some(user)) = user_db::get_user_by_id(&app_state.db, user_id).await else {
        return None;
    };

    if user.sync_url.as_deref() != Some(user_hash) {
        return None;
    }

    Some(user)
}

pub async fn download_model(
    Path(blob_sha256): Path<String>,
    State(app_state): State<AppState>,
    Query(params): Query<DownloadModelParams>,
) -> Response {
    // Each branch owns its value here so `user` can borrow from either.
    let hash_user;
    let share_scope;
    let (user, scope) = match params {
        DownloadModelParams {
            user_id: Some(user_id),
            user_hash: Some(user_hash),
            share_id: None,
        } => match extract_user_via_id_and_hash(&app_state, user_id, &user_hash).await {
            Some(user) => {
                hash_user = user;
                (&hash_user, None)
            }
            None => return StatusCode::NOT_FOUND.into_response(),
        },
        DownloadModelParams {
            user_id: None,
            user_hash: None,
            share_id: Some(share_id),
        } => match ShareScope::resolve(&app_state, &share_id).await {
            Ok(scope) => {
                share_scope = scope;
                (&share_scope.owner, Some(&share_scope))
            }
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        },
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    let Ok(Some(model_id)) =
        model_db::get_model_id_via_sha256(&app_state.db, user, &blob_sha256).await
    else {
        return StatusCode::NOT_FOUND.into_response();
    };

    // The sha256 lookup spans all of the owner's models, so a share link must
    // be checked against the share's own model ids.
    if scope.is_some_and(|scope| !scope.allows(model_id)) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let Ok(Some(model)) = model_db::get_model_via_id(&app_state.db, user, model_id).await else {
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

pub async fn get_blob_bytes(
    CurrentUser(user): CurrentUser,
    Path(sha256): Path<String>,
    State(app_state): State<AppState>,
) -> Response {
    // Resolve an owned model to authorize access to this blob.
    let Ok(Some(_model_id)) =
        model_db::get_model_id_via_sha256(&app_state.db, &user, &sha256).await
    else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let Ok(Some(blob)) = db::blob_db::get_blob_via_sha256(&app_state.db, &sha256).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    get_blob_bytes_inner(&blob, &app_state).await
}

pub async fn get_blob_thumb(
    Path(sha256): Path<String>,
    State(app_state): State<AppState>,
) -> Result<Response, ApplicationError> {
    // The length and hex checks keep the value a single path component, preventing traversal out of the image dir.
    if sha256.len() != 64
        || !sha256
            .chars()
            .all(|character: char| character.is_ascii_hexdigit())
    {
        return Ok(StatusCode::BAD_REQUEST.into_response());
    }

    let base_dir = app_state.get_image_dir();
    let src_file_path = match resolve_path_under_base(&base_dir, &format!("{sha256}.png")).await {
        Ok(path) => path,
        Err(e) => return e.respond(),
    };

    let Ok(file) = File::open(src_file_path).await else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    Ok(Body::from_stream(ReaderStream::with_capacity(file, STREAM_CHUNK_SIZE)).into_response())
}

/// Streams a blob's content; shared by `download_model` and `get_blob_bytes`.
async fn get_blob_bytes_inner(blob: &Blob, app_state: &AppState) -> Response {
    // The plain-file vs first-zip-entry convention lives in the service layer.
    let Ok(reader) = export_service::open_blob_content_reader(blob, app_state).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    Body::from_stream(ReaderStream::with_capacity(reader, STREAM_CHUNK_SIZE)).into_response()
}

pub async fn get_blobs_zip_download(
    Path(zip_dir): Path<String>,
) -> Result<Response, ApplicationError> {
    // Axum percent-decodes the segment, so `%2F` would otherwise let a
    // prefixed name reach a sibling dir via `..`; require one path component.
    if !zip_dir.starts_with(export_service::TEMP_DIR_PREFIX) || zip_dir.contains(['/', '\\']) {
        return Ok(StatusCode::BAD_REQUEST.into_response());
    }

    let path = match resolve_path_under_base(&std::env::temp_dir(), &zip_dir).await {
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

    let file = match File::open(zip_file.path()).await {
        Ok(file) => file,
        Err(e) => return crate::path_safety::OpenUnderBaseError::from(e).respond(),
    };
    let mut response =
        Body::from_stream(ReaderStream::with_capacity(file, STREAM_CHUNK_SIZE)).into_response();

    response.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{name_lossy}\"")
            .parse()
            .unwrap(),
    );

    Ok(response)
}

pub async fn create_blobs_zip_download(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
    Json(blob_sha256s): Json<Vec<String>>,
) -> Result<Response, ApplicationError> {
    let Ok(model_ids) =
        model_db::get_model_ids_via_sha256s(&app_state.db, &user, &blob_sha256s).await
    else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    let Ok(models) = model_db::get_models_via_ids(&app_state.db, &user, model_ids).await else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    if models.len() != blob_sha256s.len() {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    let path = export_service::export_zip_to_temp_folder(models, &app_state).await?;

    Ok(Json(
        path.temp_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string(),
    )
    .into_response())
}

#[cfg(test)]
mod tests {
    fn zip_download_dir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(service::export_service::TEMP_DIR_PREFIX)
            .tempdir()
            .unwrap()
    }

    async fn download_response(dir: &std::path::Path) -> axum::response::Response {
        let zip_dir = dir.file_name().unwrap().to_str().unwrap().to_string();

        axum::response::IntoResponse::into_response(
            super::get_blobs_zip_download(axum::extract::Path(zip_dir)).await,
        )
    }

    #[tokio::test]
    async fn zip_download_streams_file_with_attachment_name() {
        let dir = zip_download_dir();
        let contents = b"zip contents";
        std::fs::write(dir.path().join("export.zip"), contents).unwrap();

        let response = download_response(dir.path()).await;

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_DISPOSITION],
            "attachment; filename=\"export.zip\""
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), contents);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn zip_download_returns_not_found_when_file_disappears() {
        let dir = zip_download_dir();
        // read_dir still sees the entry, but opening its missing target returns NotFound.
        std::os::unix::fs::symlink("missing.zip", dir.path().join("export.zip")).unwrap();

        let response = download_response(dir.path()).await;

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn zip_download_keeps_other_open_errors_as_internal_server_error() {
        let dir = zip_download_dir();
        std::os::unix::fs::symlink("export.zip", dir.path().join("export.zip")).unwrap();

        let response = download_response(dir.path()).await;

        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
