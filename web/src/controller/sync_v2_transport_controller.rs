//! Opt-in test router. It is deliberately not mounted by the production app.
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};

use service::AppState;

use crate::user::CurrentSyncUser;
#[path = "sync_observation_transport.rs"]
mod observation;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/sync/handshake", post(handshake))
        .route("/api/v2/sync/identity", get(identity))
        .route("/api/v2/sync/items", get(list).post(publish))
        .route(
            "/api/v2/sync/observations",
            post(observation::publish)
                .layer(axum::extract::DefaultBodyLimit::max(66 * 1024 * 1024)),
        )
        .route(
            "/api/v2/sync/observations/{sequence}/blobs/{key}",
            get(observation::download),
        )
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityQuery {
    protocol: u32,
}
async fn identity(
    CurrentSyncUser(user, account): CurrentSyncUser,
    session: tower_sessions::Session,
    State(state): State<AppState>,
    Query(query): Query<IdentityQuery>,
) -> Response {
    if !current(&session, query.protocol).await {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    }
    match db::user_db::get_sync_identity(&state.db, user.id).await {
        Ok(Some(identity)) if identity.account_id == account => {
            Json(serde_json::json!({"protocol":1,"server_identity":identity})).into_response()
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Handshake {
    protocol: u32,
    client_identity: db::user_db::SyncIdentity,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    protocol: u32,
    writer_nonce: Option<String>,
    connection_id: String,
    item: serde_json::Value,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    protocol: u32,
    connection_id: String,
    after: String,
    through: Option<String>,
    limit: usize,
}
async fn current(session: &tower_sessions::Session, protocol: u32) -> bool {
    protocol == 1
        && matches!(
            session
                .get::<u32>(crate::user::SESSION_GROUP_DELETION_PROTOCOL)
                .await,
            Ok(Some(1))
        )
}
fn secret(headers: &HeaderMap) -> &str {
    headers
        .get("x-sync-writer-secret")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
}
fn response<T: serde::Serialize>(result: Result<T, db::DbError>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(db::DbError::Protocol(message)) => {
            let reason = message
                .strip_prefix("sync_transport:")
                .unwrap_or("identity_blocked");
            if let Some(encoded) = reason.strip_prefix("needs_dependencies:") {
                let Ok(missing) =
                    serde_json::from_str::<Vec<db::causal_envelope::Dependency>>(encoded)
                else {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                };

                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({"status":"needs_dependencies","missing":missing})),
                )
                    .into_response();
            }
            let status = match reason {
                "unauthorized" => StatusCode::UNAUTHORIZED,
                "forbidden" => StatusCode::FORBIDDEN,
                "too_large" => StatusCode::PAYLOAD_TOO_LARGE,
                _ => StatusCode::CONFLICT,
            };
            (status, Json(serde_json::json!({"status":reason}))).into_response()
        }
        Err(error) => crate::error::ApplicationError::from(error).into_response(),
    }
}
async fn handshake(
    CurrentSyncUser(user, account): CurrentSyncUser,
    session: tower_sessions::Session,
    State(state): State<AppState>,
    Json(body): Json<Handshake>,
) -> Response {
    if !current(&session, body.protocol).await {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    }
    response(
        db::sync_transport::register_writer(&state.db, &user, &account, &body.client_identity)
            .await,
    )
}
async fn publish(
    CurrentSyncUser(user, account): CurrentSyncUser,
    session: tower_sessions::Session,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Publish>,
) -> Response {
    if !current(&session, body.protocol).await {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    }
    response(
        db::sync_transport::publish(
            &state.db,
            &user,
            &account,
            &body.connection_id,
            secret(&headers),
            body.writer_nonce.as_deref(),
            body.item,
        )
        .await,
    )
}
fn cursor(value: &str) -> Option<i64> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    value.parse().ok()
}
async fn list(
    CurrentSyncUser(user, account): CurrentSyncUser,
    session: tower_sessions::Session,
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(body): Query<List>,
) -> Response {
    if !current(&session, body.protocol).await {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    }
    let Some(after) = cursor(&body.after) else {
        return StatusCode::CONFLICT.into_response();
    };
    let through = match body.through {
        Some(value) => match cursor(&value) {
            Some(value) => Some(value),
            None => return StatusCode::CONFLICT.into_response(),
        },
        None => None,
    };
    response(
        db::sync_transport::list(
            &state.db,
            &user,
            &account,
            &body.connection_id,
            secret(&headers),
            after,
            through,
            body.limit,
        )
        .await,
    )
}
#[cfg(test)]
#[path = "sync_v2_transport_tests.rs"]
mod tests;
