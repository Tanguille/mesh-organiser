use std::{
    env,
    error::Error,
    fmt::Write,
    fs::File,
    io::{self, ErrorKind},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    http::HeaderValue,
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::any,
};
use axum_login::AuthManagerLayerBuilder;
use time::{Duration, OffsetDateTime};
use tokio::{fs, signal, task::AbortHandle};
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer},
    services::{ServeDir, ServeFile},
};
use tower_sessions::{ExpiredDeletion, Expiry, SessionManagerLayer, cookie::Key};

use db::{
    db_context::{self, DbContext},
    model::user::User,
    user_db,
};
use service::{
    AppState, Configuration, export_service, import_state::ImportState, thumbnail_service,
};

use crate::{
    controller::{
        auth_controller, blob_controller, group_controller, label_controller, model_controller,
        page_controller, resource_controller, share_controller, slicer_controller,
        threemf_controller, user_controller,
    },
    session_store::SqliteStore,
    user::{AuthSession, Backend},
    web_import_state::WebImportStateEmitter,
};

// Env var names live in one place so a lookup and the message naming it in an
// error cannot drift apart.
const ENV_SERVER_PORT: &str = "SERVER_PORT";
const ENV_LOCAL_ACCOUNT_PASSWORD: &str = "LOCAL_ACCOUNT_PASSWORD";
const ENV_REGENERATE_THUMBNAILS: &str = "REGENERATE_THUMBNAILS";
const ENV_APP_CONFIG_PATH: &str = "APP_CONFIG_PATH";

pub struct App {
    state: AppState,
    session_store: SqliteStore,
}

/// Origins always allowed for local dev; [`merge_cors_origins_from_env`] may add more.
const DEFAULT_CORS_ORIGINS: &[&str] = &["http://localhost:3000", "http://localhost:5173"];

/// Builds the allowed CORS origin list: defaults plus optional `MESH_ORGANISER_CORS_ORIGINS`
/// (comma-separated, trimmed). If the variable is set and every non-empty token is invalid,
/// returns an error so startup fails clearly.
fn merge_cors_origins_from_env(
    env_value: Option<&str>,
) -> Result<Vec<HeaderValue>, Box<dyn Error>> {
    let mut origins: Vec<HeaderValue> = DEFAULT_CORS_ORIGINS
        .iter()
        .map(|s| {
            s.parse::<HeaderValue>().map_err(|e| {
                io::Error::other(format!("internal: invalid default CORS origin {s:?}: {e}"))
            })
        })
        .collect::<Result<_, _>>()?;

    let Some(raw) = env_value else {
        return Ok(origins);
    };

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(origins);
    }

    let mut saw_nonempty_token = false;
    let mut saw_valid_token = false;
    for part in trimmed.split(',') {
        let t = part.trim();
        if t.is_empty() {
            continue;
        }
        saw_nonempty_token = true;
        match t.parse::<HeaderValue>() {
            Ok(h) => {
                saw_valid_token = true;
                if !origins.contains(&h) {
                    origins.push(h);
                }
            }
            Err(e) => {
                eprintln!("MESH_ORGANISER_CORS_ORIGINS: skipping invalid origin {t:?}: {e}");
            }
        }
    }

    if saw_nonempty_token && !saw_valid_token {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "MESH_ORGANISER_CORS_ORIGINS: all entries were invalid after trimming; \
             fix the value or unset the variable to use defaults only",
        )
        .into());
    }

    Ok(origins)
}

fn cors_layer_from_env() -> Result<CorsLayer, Box<dyn Error>> {
    let extra = env::var("MESH_ORGANISER_CORS_ORIGINS").ok();
    let origins = merge_cors_origins_from_env(extra.as_deref())?;
    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        // Wildcards are illegal (and panic in tower-http) together with
        // allow_credentials; mirroring the request is the permissive-but-legal
        // equivalent for credentialed CORS.
        .allow_methods(AllowMethods::mirror_request())
        .allow_headers(AllowHeaders::mirror_request())
        .allow_credentials(true))
}

pub fn parse_port() -> Result<u16, Box<dyn Error>> {
    env::var(ENV_SERVER_PORT)
        .unwrap_or_else(|_| "3000".into())
        .parse::<u16>()
        .map_err(|e| {
            io::Error::new(
                ErrorKind::InvalidInput,
                format!("SERVER_PORT must be a valid u16: {e}"),
            )
            .into()
        })
}

fn ensure_config_file_exists(config_path: &Path) -> Result<(), Box<dyn Error>> {
    if !config_path.exists() {
        let mut config_file = File::create(config_path)?;
        io::Write::write_all(
            &mut config_file,
            serde_json::to_string_pretty(&Configuration::default())
                .expect("default configuration is serializable")
                .as_bytes(),
        )?;
    }
    Ok(())
}

async fn load_and_prepare_config(config_path: &Path) -> Result<Configuration, Box<dyn Error>> {
    let json = fs::read_to_string(config_path).await.map_err(|e| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!("Failed to read configuration: {e}"),
        )
    })?;
    let mut configuration: Configuration = serde_json::from_str(&json).map_err(|e| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!("Failed to parse configuration: {e}"),
        )
    })?;
    if configuration.data_path.is_empty() {
        let default_data_dir = config_path.parent().ok_or_else(|| {
            io::Error::new(
                ErrorKind::InvalidData,
                "APP_CONFIG_PATH has no parent directory",
            )
        })?;
        configuration.data_path = default_data_dir
            .to_str()
            .ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidData, "config path is not valid UTF-8")
            })?
            .to_string();
    }
    Ok(configuration)
}

/// Resets the local (user 1) account to a fresh random password and scrambles its
/// validity token so no credential from a previous run survives. Uses
/// `ENV_LOCAL_ACCOUNT_PASSWORD` when set. Also drops dead groups and blobs left
/// by skipped or interrupted cleanup.
async fn apply_local_account(app_state: &AppState) -> Result<(), Box<dyn Error>> {
    let local_pass = env::var(ENV_LOCAL_ACCOUNT_PASSWORD).unwrap_or_else(|_| {
        let key = Key::generate();
        let key_bytes = key.master();
        let mut pass = String::new();
        for b in key_bytes {
            write!(pass, "{b:02X}").expect("write to String is infallible");
        }

        pass
    });
    user_db::edit_user_password(&app_state.db, 1, &local_pass).await?;
    user_db::scramble_validity_token(&app_state.db, 1).await?;
    // Expire models before the group/blob sweeps, including blobs a user deletion deferred;
    // nothing holds the import lock yet at this point.
    export_service::cleanup_library_on_startup(app_state).await?;

    Ok(())
}

/// Removes stale temp dirs and purges expired trash (with the blobs that leaves dead)
/// right away, then every hour. The first blob sweep may wait for thumbnail
/// regeneration's import lock without blocking app startup because it runs in the background.
fn spawn_hourly_cleanup(app_state: &AppState) {
    let app_state = app_state.clone();
    tokio::spawn(async move {
        // The first tick completes immediately.
        let mut interval = tokio::time::interval(tokio::time::Duration::from_hours(1));
        loop {
            interval.tick().await;
            let _ = export_service::remove_stale_temp_dirs();

            if let Err(e) = export_service::purge_expired_trash(&app_state).await {
                eprintln!("Purging expired trash failed: {e}");
            }
        }
    });
}

/// Spawns the `REGENERATE_THUMBNAILS=all|missing` pass as a background task so
/// the TCP listener can bind immediately; a large library's CPU-bound thumbnail
/// pass would otherwise keep the server unreachable until it finishes.
fn spawn_thumbnail_regeneration(app_state: &AppState) {
    let regenerate_thumbnails = env::var(ENV_REGENERATE_THUMBNAILS)
        .unwrap_or_else(|_| "none".into())
        .to_lowercase();
    let force = match regenerate_thumbnails.as_str() {
        "all" => true,
        "missing" => false,
        _ => return,
    };

    let app_state = app_state.clone();
    tokio::spawn(async move {
        // Hold the import mutex so regeneration does not race concurrent imports.
        let _import_guard = app_state.import_mutex.clone().lock_owned().await;

        println!(
            "Regenerating {} thumbnails...",
            if force { "all" } else { "missing" }
        );
        let mut import_state = ImportState::new_with_emitter(
            None,
            false,
            true,
            false,
            User::default(),
            Box::new(WebImportStateEmitter {}),
        );
        if let Err(e) =
            thumbnail_service::generate_all_thumbnails(&app_state, force, &mut import_state).await
        {
            eprintln!("Thumbnail regeneration failed: {e}");
        }
    });
}

async fn update_session_middleware(
    auth_session: AuthSession,
    request: Request,
    next: Next,
) -> Response {
    if auth_session.user.is_some() {
        let expiry_date = auth_session.session.expiry_date();
        let now = OffsetDateTime::now_utc();
        let difference = expiry_date - now;

        if difference < Duration::days(5) {
            auth_session
                .session
                .set_expiry(Some(Expiry::OnInactivity(Duration::days(7))));
        }
    }

    next.run(request).await
}

impl App {
    pub async fn new() -> Result<Self, Box<dyn Error>> {
        let config_path = env::var(ENV_APP_CONFIG_PATH)
            .map_err(|_| {
                io::Error::new(
                    ErrorKind::NotFound,
                    format!("Expected environment variable {ENV_APP_CONFIG_PATH} to be set"),
                )
            })
            .map(PathBuf::from)?;

        ensure_config_file_exists(&config_path)?;
        let configuration = load_and_prepare_config(&config_path).await?;

        let data_dir = PathBuf::from(&configuration.data_path);
        let sqlite_path = data_dir.join("db.sqlite");
        let sqlite_backup_dir = data_dir.join("backups");
        let db = db_context::setup_db(&sqlite_path, &sqlite_backup_dir).await;

        let app_state = AppState {
            db: Arc::new(db),
            configuration: Mutex::new(configuration),
            app_data_path: data_dir
                .to_str()
                .ok_or_else(|| {
                    io::Error::new(ErrorKind::InvalidData, "data_path is not valid UTF-8")
                })?
                .to_string(),
            import_mutex: Arc::new(tokio::sync::Mutex::new(())),
        };

        let session_store = SqliteStore::new(app_state.db.as_ref().clone());
        session_store.migrate().await?;

        apply_local_account(&app_state).await?;
        spawn_thumbnail_regeneration(&app_state);
        spawn_hourly_cleanup(&app_state);

        Ok(Self {
            state: app_state,
            session_store,
        })
    }

    /// `port` is passed in rather than stored on `App`, keeping it apart from the state.
    pub async fn serve(self, port: u16) -> Result<(), Box<dyn Error>> {
        let session_store = self.session_store;

        let deletion_task = tokio::task::spawn(
            session_store
                .clone()
                .continuously_delete_expired(tokio::time::Duration::from_mins(1)),
        );

        let signing_key_path = PathBuf::from(&self.state.app_data_path).join("signing.key");
        let key = if signing_key_path.exists() {
            let key_bytes = fs::read(&signing_key_path).await?;
            Key::from(&key_bytes)
        } else {
            let key = Key::generate();
            fs::write(&signing_key_path, key.master()).await?;
            key
        };

        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(true)
            .with_expiry(Expiry::OnInactivity(Duration::days(7)))
            .with_signed(key);

        let backend = Backend::new(self.state.db.clone());
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        // Unknown paths are client-side routes, so they get the SPA shell with a 200. Build assets
        // under `/_app` are served apart below, so a missing one stays a 404 that a CDN will not
        // cache as JS.
        let serve_dir = ServeDir::new("www").fallback(ServeFile::new("www/index.html"));
        let db = self.state.db.clone();

        let cors_layer = cors_layer_from_env()?;

        let governor_config = Arc::new(
            GovernorConfigBuilder::default()
                .per_second(5)
                .burst_size(10)
                .finish()
                .ok_or_else(|| {
                    io::Error::new(ErrorKind::InvalidData, "Failed to create governor config")
                })?,
        );

        let login_router =
            auth_controller::login_router().layer(GovernorLayer::new(governor_config));

        // In each controller's router, `login_required!` is a route layer, so it guards only
        // the routes registered before it; public routes are registered after it.
        let app = Router::new()
            .merge(login_router)
            .merge(auth_controller::router())
            .merge(blob_controller::router())
            .merge(model_controller::router())
            .merge(group_controller::router())
            .merge(label_controller::router())
            .merge(resource_controller::router())
            .merge(user_controller::router())
            .merge(threemf_controller::router())
            .merge(page_controller::router())
            .merge(share_controller::router())
            .merge(slicer_controller::router())
            // Unknown API paths stay a 404 instead of falling through to the SPA shell below.
            .route("/api/{*path}", any(|| async { StatusCode::NOT_FOUND }))
            // So do discovery paths: an MCP client probing `/.well-known/oauth-*` must see that
            // no OAuth server is here, not a 200 HTML page it tries to parse as metadata.
            .route(
                "/.well-known/{*path}",
                any(|| async { StatusCode::NOT_FOUND }),
            )
            .nest_service("/_app", ServeDir::new("www/_app"))
            .with_state(self.state)
            .layer(cors_layer)
            .layer(middleware::from_fn(update_session_middleware))
            .layer(auth_layer)
            .layer(DefaultBodyLimit::disable())
            .layer(CompressionLayer::new())
            .fallback_service(serve_dir);

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;

        println!("Server running on port {port}");

        // Connect info is required by the auth rate limiter: tower_governor's
        // default PeerIpKeyExtractor 500s every request without a peer address.
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        // The shutdown signal also aborts the session deletion task.
        .with_graceful_shutdown(shutdown_signal(deletion_task.abort_handle(), db))
        .await?;

        deletion_task.await??;

        Ok(())
    }
}

async fn shutdown_signal(deletion_task_abort_handle: AbortHandle, db: Arc<DbContext>) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => { deletion_task_abort_handle.abort() },
        () = terminate => { deletion_task_abort_handle.abort() },
    }

    db.close().await;
}

#[cfg(test)]
mod tests {
    use super::merge_cors_origins_from_env;

    #[test]
    fn cors_merge_none_is_defaults_only() {
        let o = merge_cors_origins_from_env(None).unwrap();
        assert_eq!(o.len(), 2);
    }

    #[test]
    fn cors_merge_whitespace_env_is_defaults_only() {
        let o = merge_cors_origins_from_env(Some("  \t  ")).unwrap();
        assert_eq!(o.len(), 2);
    }

    #[test]
    fn cors_merge_extends_with_extra_origin() {
        let o = merge_cors_origins_from_env(Some("http://192.168.1.10:5173")).unwrap();
        assert_eq!(o.len(), 3);
        assert!(
            o.iter()
                .any(|h| h.as_bytes() == b"http://192.168.1.10:5173")
        );
    }

    #[test]
    fn cors_merge_duplicate_extra_does_not_duplicate_list() {
        let o = merge_cors_origins_from_env(Some("http://localhost:3000")).unwrap();
        assert_eq!(o.len(), 2);
    }

    #[test]
    fn cors_merge_all_invalid_tokens_errors() {
        let err = merge_cors_origins_from_env(Some("\u{0}, \n")).unwrap_err();
        assert!(err.to_string().contains("MESH_ORGANISER_CORS_ORIGINS"));
    }

    #[test]
    fn cors_merge_one_valid_among_invalid_succeeds() {
        let o = merge_cors_origins_from_env(Some("http://10.0.0.1:8080,\u{0}")).unwrap();
        assert_eq!(o.len(), 3);
    }

    /// Adds a blob whose model file and thumbnail exist on disk, as an import leaves them.
    async fn add_blob_with_files(
        app_state: &service::AppState,
        sha256: &str,
    ) -> (i64, std::path::PathBuf, std::path::PathBuf) {
        let blob_id = db::blob_db::add_blob(&app_state.db, sha256, "stl", 1, None)
            .await
            .unwrap();
        let blob = db::blob_db::get_blob_via_sha256(&app_state.db, sha256)
            .await
            .unwrap()
            .unwrap();
        let model_path = service::export_service::get_model_path_for_blob(&blob, app_state);
        let image_path = service::export_service::get_image_path_for_blob(&blob, app_state);
        std::fs::write(&model_path, sha256).unwrap();
        std::fs::write(&image_path, sha256).unwrap();

        (blob_id, model_path, image_path)
    }

    #[tokio::test]
    async fn apply_local_account_sweeps_dead_blobs_and_keeps_referenced_ones() {
        let dir = tempfile::TempDir::new().unwrap();
        let data_path = dir.path().to_string_lossy().to_string();
        let backup_dir = dir.path().join("backups");
        std::fs::create_dir_all(&backup_dir).unwrap();
        // The migrations seed the local account as user 1, which `apply_local_account` edits.
        let db = db::db_context::setup_db(&dir.path().join("db.sqlite"), &backup_dir).await;
        let app_state = service::AppState {
            db: std::sync::Arc::new(db),
            configuration: std::sync::Mutex::new(service::Configuration {
                data_path: data_path.clone(),
                ..Default::default()
            }),
            import_mutex: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            app_data_path: data_path,
        };
        // No model references this blob, as a skipped or interrupted sweep leaves it.
        let (_, dead_model_path, dead_image_path) = add_blob_with_files(&app_state, "dead").await;
        let (kept_blob_id, kept_model_path, kept_image_path) =
            add_blob_with_files(&app_state, "kept").await;
        let local_user = db::model::user::User::default();
        db::model_db::add_model(&app_state.db, &local_user, "kept", kept_blob_id, None, None)
            .await
            .unwrap();

        super::apply_local_account(&app_state).await.unwrap();

        let dead_blob = db::blob_db::get_blob_via_sha256(&app_state.db, "dead")
            .await
            .unwrap();
        let kept_blob = db::blob_db::get_blob_via_sha256(&app_state.db, "kept")
            .await
            .unwrap();
        assert!(dead_blob.is_none(), "dead blob row survived");
        assert!(!dead_model_path.exists(), "dead blob model file survived");
        assert!(!dead_image_path.exists(), "dead blob thumbnail survived");
        assert_eq!(kept_blob.unwrap().id, kept_blob_id);
        assert!(kept_model_path.exists());
        assert!(kept_image_path.exists());
    }
}
