use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
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

/// A share link needs no login, so its zip request body is read before anything is authorised.
/// The body is capped at what the most models one zip can hold (`get_models` returns at most
/// `MAX_PAGE_SIZE`) take as JSON sha256s, with room to spare.
const SHARE_ZIP_BODY_LIMIT: usize = db::MAX_PAGE_SIZE as usize * 64;

pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/blobs/{sha256}/bytes", get(get_blob_bytes))
            .route("/blobs/download", post(create_blobs_zip_download))
            .route_layer(login_required!(Backend))
            .route("/blobs/{sha256}/thumb", get(get_blob_thumb))
            .route("/blobs/{sha256}/download", get(download_model))
            .route("/blobs/download/{zip_dir}", get(get_blobs_zip_download))
            .route(
                "/shares/{share_id}/blobs/download",
                post(create_share_blobs_zip_download)
                    .layer(DefaultBodyLimit::max(SHARE_ZIP_BODY_LIMIT)),
            ),
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
    // The stored-hash shape also keeps the value a single path component, so it cannot traverse
    // out of the image dir.
    if !db::is_blob_sha256(&sha256) {
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
    create_blobs_zip_download_inner(&app_state, &user, None, &blob_sha256s).await
}

/// Lets a share link open several models in a slicer, which fetches the zip without a session.
pub async fn create_share_blobs_zip_download(
    Path(share_id): Path<String>,
    State(app_state): State<AppState>,
    Json(blob_sha256s): Json<Vec<String>>,
) -> Result<Response, ApplicationError> {
    let scope = ShareScope::resolve(&app_state, &share_id).await?;

    create_blobs_zip_download_inner(&app_state, &scope.owner, Some(&scope), &blob_sha256s).await
}

/// Zips `user`'s models with these blobs and answers the temp dir name that
/// `get_blobs_zip_download` serves; the hourly temp dir sweep removes it once stale.
async fn create_blobs_zip_download_inner(
    app_state: &AppState,
    user: &User,
    scope: Option<&ShareScope>,
    blob_sha256s: &[String],
) -> Result<Response, ApplicationError> {
    let Ok(model_ids) =
        model_db::get_model_ids_via_sha256s(&app_state.db, user, blob_sha256s).await
    else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    // The sha256 lookup spans all of the owner's models, so a share link must
    // be checked against the share's own model ids.
    if scope.is_some_and(|scope| !model_ids.iter().all(|model_id| scope.allows(*model_id))) {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    let Ok(models) = model_db::get_models_via_ids(&app_state.db, user, model_ids).await else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    if models.len() != blob_sha256s.len() {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    let path = export_service::export_zip_to_temp_folder(models, app_state).await?;

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

    /// The thumbnail handler only reads the image dir, so the pool is lazy and never connects.
    fn app_state(dir: &tempfile::TempDir) -> service::AppState {
        let data_path = dir.path().to_string_lossy().to_string();

        service::AppState {
            db: std::sync::Arc::new(sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap()),
            configuration: std::sync::Mutex::new(service::Configuration {
                data_path: data_path.clone(),
                ..Default::default()
            }),
            import_mutex: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            app_data_path: data_path,
        }
    }

    /// Called directly rather than through the router: axum's `Path` never carries a `/` in one
    /// segment, and the handler must still refuse one.
    async fn thumb(app_state: &service::AppState, sha256: &str) -> axum::response::Response {
        let result = super::get_blob_thumb(
            axum::extract::Path(sha256.to_string()),
            axum::extract::State(app_state.clone()),
        )
        .await;

        axum::response::IntoResponse::into_response(result)
    }

    #[tokio::test]
    async fn get_blob_thumb_serves_the_png_of_a_stored_32_hex_sha256() {
        let dir = tempfile::TempDir::new().unwrap();
        let app_state = app_state(&dir);
        let sha256 = db::u128_to_hex_32(0xabc);
        let png = b"\x89PNG\r\n\x1a\nthumbnail".to_vec();
        std::fs::write(
            app_state.get_image_dir().join(format!("{sha256}.png")),
            &png,
        )
        .unwrap();

        let response = thumb(&app_state, &sha256).await;

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body, png);
    }

    #[tokio::test]
    async fn get_blob_thumb_is_not_found_without_a_png() {
        let dir = tempfile::TempDir::new().unwrap();
        let app_state = app_state(&dir);

        let response = thumb(&app_state, &db::u128_to_hex_32(0xabc)).await;

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    /// A local-account (user 1) model whose file exists on disk, as an import leaves it.
    async fn add_model_with_file(app_state: &service::AppState, name: &str, sha256: &str) -> i64 {
        let blob_id = db::blob_db::add_blob(&app_state.db, sha256, "stl", 1, None)
            .await
            .unwrap();
        let blob = db::blob_db::get_blob_via_sha256(&app_state.db, sha256)
            .await
            .unwrap()
            .unwrap();
        std::fs::write(
            service::export_service::get_model_path_for_blob(&blob, app_state),
            name,
        )
        .unwrap();

        db::model_db::add_model(
            &app_state.db,
            &db::model::user::User::default(),
            name,
            blob_id,
            None,
            None,
        )
        .await
        .unwrap()
    }

    /// A share of the local account's models `boat` and `raft`, next to its unshared `dinghy`.
    async fn shared_library() -> (tempfile::TempDir, service::AppState, String) {
        let (dir, app_state) = crate::controller::share_controller::tests::app_state().await;
        let owner = db::model::user::User::default();
        let boat_id = add_model_with_file(&app_state, "boat", &db::u128_to_hex_32(1)).await;
        let raft_id = add_model_with_file(&app_state, "raft", &db::u128_to_hex_32(2)).await;
        add_model_with_file(&app_state, "dinghy", &db::u128_to_hex_32(3)).await;
        let share_id = db::share_db::create_share(&app_state.db, &owner, "fleet")
            .await
            .unwrap();
        db::share_db::set_model_ids_on_share(
            &app_state.db,
            &owner,
            &share_id,
            vec![boat_id, raft_id],
        )
        .await
        .unwrap();

        (dir, app_state, share_id)
    }

    /// Sent without a session, as a slicer opening a share link does, through the body limit
    /// that `app.rs` lifts for every route.
    async fn send(
        app_state: &service::AppState,
        request: axum::http::Request<axum::body::Body>,
    ) -> axum::response::Response {
        let router = super::router()
            .with_state(app_state.clone())
            .layer(axum::extract::DefaultBodyLimit::disable());

        tower::ServiceExt::oneshot(router, request).await.unwrap()
    }

    async fn post_share_zip(
        app_state: &service::AppState,
        share_id: &str,
        sha256s: &[u128],
    ) -> axum::response::Response {
        let sha256s: Vec<String> = sha256s.iter().copied().map(db::u128_to_hex_32).collect();
        let request =
            axum::http::Request::post(format!("/api/v1/shares/{share_id}/blobs/download"))
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(
                    serde_json::to_vec(&sha256s).unwrap(),
                ))
                .unwrap();

        send(app_state, request).await
    }

    #[tokio::test]
    async fn share_zip_download_serves_the_shared_models_without_a_session() {
        let (_dir, app_state, share_id) = shared_library().await;

        let response = post_share_zip(&app_state, &share_id, &[1, 2]).await;

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let zip_dir: String = serde_json::from_slice(&body).unwrap();
        let download = send(
            &app_state,
            axum::http::Request::get(format!("/api/v1/blobs/download/{zip_dir}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
        let status = download.status();
        let zip = axum::body::to_bytes(download.into_body(), usize::MAX)
            .await
            .unwrap();
        std::fs::remove_dir_all(std::env::temp_dir().join(&zip_dir)).unwrap();
        assert_eq!(status, axum::http::StatusCode::OK);
        let archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
        let mut names: Vec<String> = archive
            .file_names()
            .map(|name| name.unwrap().into_owned())
            .collect();
        names.sort_unstable();
        assert_eq!(names, ["boat.stl", "raft.stl"]);
    }

    #[tokio::test]
    async fn share_zip_download_refuses_a_model_outside_the_share() {
        let (_dir, app_state, share_id) = shared_library().await;

        let response = post_share_zip(&app_state, &share_id, &[1, 3]).await;

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn share_zip_download_is_not_found_for_an_unknown_share() {
        let (_dir, app_state, _share_id) = shared_library().await;

        let response = post_share_zip(&app_state, "missing", &[1, 2]).await;

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn share_zip_download_refuses_a_trashed_shared_model() {
        let (_dir, app_state, share_id) = shared_library().await;
        let boat_id = db::model_db::get_model_id_via_sha256(
            &app_state.db,
            &db::model::user::User::default(),
            &db::u128_to_hex_32(1),
        )
        .await
        .unwrap()
        .unwrap();
        db::model_db::trash_models(&app_state.db, &db::model::user::User::default(), &[boat_id])
            .await
            .unwrap();

        let response = post_share_zip(&app_state, &share_id, &[1, 2]).await;

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn share_zip_download_caps_the_body_but_fits_a_full_zip_request() {
        let (_dir, app_state, share_id) = shared_library().await;
        let full_request = vec![db::u128_to_hex_32(u128::MAX); db::MAX_PAGE_SIZE as usize];
        let request =
            axum::http::Request::post(format!("/api/v1/shares/{share_id}/blobs/download"))
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(
                    " ".repeat(super::SHARE_ZIP_BODY_LIMIT + 1),
                ))
                .unwrap();

        let response = send(&app_state, request).await;

        assert_eq!(response.status(), axum::http::StatusCode::PAYLOAD_TOO_LARGE);
        assert!(serde_json::to_vec(&full_request).unwrap().len() <= super::SHARE_ZIP_BODY_LIMIT);
    }

    #[tokio::test]
    async fn get_blob_thumb_rejects_a_value_that_is_not_all_hex() {
        let dir = tempfile::TempDir::new().unwrap();
        let app_state = app_state(&dir);
        let non_hex = "z".repeat(32);

        for sha256 in ["../etc", "abc/def", non_hex.as_str(), ""] {
            let response = thumb(&app_state, sha256).await;

            assert_eq!(
                response.status(),
                axum::http::StatusCode::BAD_REQUEST,
                "{sha256:?}"
            );
        }
    }
}
