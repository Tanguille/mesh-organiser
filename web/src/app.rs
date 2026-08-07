use std::{
    env,
    fmt::Write,
    fs::File,
    io::{self, ErrorKind},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use axum::{
    extract::{DefaultBodyLimit, Request},
    http::HeaderValue,
    middleware::{self, Next},
    response::Response,
};
use axum_login::AuthManagerLayerBuilder;
use axum_messages::MessagesManagerLayer;
use time::{Duration, OffsetDateTime};
use tokio::{fs, signal, task::AbortHandle};
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer},
    services::{ServeDir, ServeFile},
};
use tower_sessions::{ExpiredDeletion, Expiry, SessionManagerLayer, cookie::Key};
use tower_sessions_sqlx_store::{
    SqliteStore,
    sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use db::{
    db_context::{self, DbContext},
    group_db,
    model::user::User,
    user_db,
};
use service::{AppState, Configuration, import_state::ImportState, thumbnail_service};

use crate::{
    controller::api_router,
    user::{AuthSession, Backend},
    web_app_state::WebAppState,
    web_import_state::WebImportStateEmitter,
};

pub struct App {
    app_state: WebAppState,
    session_store: SqliteStore,
}

fn expected_env_error_msg(var_name: &str) -> String {
    format!("Expected environment variable {var_name} to be set")
}

/// Origins always allowed for local dev; [`merge_cors_origins_from_env`] may add more.
const DEFAULT_CORS_ORIGINS: &[&str] = &["http://localhost:3000", "http://localhost:5173"];

/// Builds the allowed CORS origin list: defaults plus optional `MESH_ORGANISER_CORS_ORIGINS`
/// (comma-separated, trimmed). If the variable is set and every non-empty token is invalid,
/// returns an error so startup fails clearly.
fn merge_cors_origins_from_env(
    env_value: Option<&str>,
) -> Result<Vec<HeaderValue>, Box<dyn std::error::Error>> {
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

fn cors_layer_from_env() -> Result<CorsLayer, Box<dyn std::error::Error>> {
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

fn parse_port() -> Result<u16, Box<dyn std::error::Error>> {
    env::var("SERVER_PORT")
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

fn ensure_config_file_exists(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
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

async fn load_and_prepare_config(
    config_path: &Path,
) -> Result<Configuration, Box<dyn std::error::Error>> {
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

async fn apply_local_account(
    web_app_state: &WebAppState,
) -> Result<(), Box<dyn std::error::Error>> {
    let local_pass = env::var("LOCAL_ACCOUNT_PASSWORD").unwrap_or_else(|_| {
        let key = Key::generate();
        let key_bytes = key.master();
        let mut pass = String::new();
        for b in key_bytes {
            write!(pass, "{b:02X}").expect("write to String is infallible");
        }

        pass
    });
    user_db::edit_user_password(&web_app_state.app_state.db, 1, &local_pass).await?;
    user_db::scramble_validity_token(&web_app_state.app_state.db, 1).await?;
    group_db::delete_dead_groups(&web_app_state.app_state.db).await?;

    Ok(())
}

/// Spawns the `REGENERATE_THUMBNAILS=all|missing` pass as a background task so
/// the TCP listener can bind immediately; a large library's CPU-bound thumbnail
/// pass would otherwise keep the server unreachable until it finishes.
fn spawn_thumbnail_regeneration(web_app_state: &WebAppState) {
    let regenerate_thumbnails = env::var("REGENERATE_THUMBNAILS")
        .unwrap_or_else(|_| "none".into())
        .to_lowercase();
    let force = match regenerate_thumbnails.as_str() {
        "all" => true,
        "missing" => false,
        _ => return,
    };

    let app_state = web_app_state.app_state.clone();
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

async fn setup_session_store(
    sqlite_path: &Path,
) -> Result<SqliteStore, Box<dyn std::error::Error>> {
    let connect_options = SqliteConnectOptions::new()
        .filename(sqlite_path)
        .create_if_missing(false)
        .busy_timeout(std::time::Duration::from_secs(15));
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(connect_options)
        .await?;
    let store = SqliteStore::new(pool);
    store.migrate().await?;

    Ok(store)
}

impl App {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let port = parse_port()?;
        let config_path = env::var("APP_CONFIG_PATH")
            .map_err(|_| {
                io::Error::new(
                    ErrorKind::NotFound,
                    expected_env_error_msg("APP_CONFIG_PATH"),
                )
            })
            .map(PathBuf::from)?;

        ensure_config_file_exists(&config_path)?;
        let configuration = load_and_prepare_config(&config_path).await?;

        let data_dir = PathBuf::from(&configuration.data_path);
        let sqlite_path = data_dir.join("db.sqlite");
        let sqlite_backup_dir = data_dir.join("backups");
        let db = db_context::setup_db(&sqlite_path, &sqlite_backup_dir).await;

        let web_app_state = WebAppState {
            app_state: AppState {
                db: Arc::new(db),
                configuration: Mutex::new(configuration),
                app_data_path: data_dir
                    .to_str()
                    .ok_or_else(|| {
                        io::Error::new(ErrorKind::InvalidData, "data_path is not valid UTF-8")
                    })?
                    .to_string(),
                import_mutex: Arc::new(tokio::sync::Mutex::new(())),
            },
            port,
        };

        let session_store = setup_session_store(&sqlite_path).await?;

        apply_local_account(&web_app_state).await?;
        spawn_thumbnail_regeneration(&web_app_state);

        Ok(Self {
            app_state: web_app_state,
            session_store,
        })
    }

    pub async fn serve(self) -> Result<(), Box<dyn std::error::Error>> {
        // Session layer.
        //
        // This uses `tower-sessions` to establish a layer that will provide the session
        // as a request extension.
        let session_store = self.session_store;

        let deletion_task = tokio::task::spawn(
            session_store
                .clone()
                .continuously_delete_expired(tokio::time::Duration::from_mins(1)),
        );

        let signing_key_path = self.app_state.get_signing_key_path();
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

        // Auth service.
        //
        // This combines the session layer with our backend to establish the auth
        // service which will provide the auth session as a request extension.
        let backend = Backend::new(self.app_state.app_state.db.clone());
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        let serve_dir = ServeDir::new("www").not_found_service(ServeFile::new("www/index.html"));
        let db = self.app_state.app_state.db.clone();
        let port = self.app_state.port;

        let cors_layer = cors_layer_from_env()?;

        let app = api_router::merged_api_router()?
            .with_state(self.app_state)
            .layer(cors_layer)
            .layer(middleware::from_fn(update_session_middleware))
            .layer(MessagesManagerLayer)
            .layer(auth_layer)
            .layer(DefaultBodyLimit::disable())
            .layer(CompressionLayer::new())
            .fallback_service(serve_dir);

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;

        println!("Server running on port {port}");

        // Ensure we use a shutdown signal to abort the deletion task.
        // Connect info is required by the auth rate limiter: tower_governor's
        // default PeerIpKeyExtractor 500s every request without a peer address.
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
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
    use super::{expected_env_error_msg, merge_cors_origins_from_env};

    #[test]
    fn expected_env_error_msg_format() {
        assert_eq!(
            expected_env_error_msg("FOO"),
            "Expected environment variable FOO to be set"
        );
        assert_eq!(
            expected_env_error_msg("APP_CONFIG_PATH"),
            "Expected environment variable APP_CONFIG_PATH to be set"
        );
    }

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
}
