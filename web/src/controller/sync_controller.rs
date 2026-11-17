use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use axum_login::login_required;
use service::AppState;

use crate::{
    error::ApplicationError,
    user::{Backend, CurrentSyncUser},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/sync/snapshot", get(get_snapshot))
        .route_layer(login_required!(Backend))
}

async fn get_snapshot(
    CurrentSyncUser(user, account_id): CurrentSyncUser,
    State(state): State<AppState>,
) -> Response {
    match db::sync_snapshot::get_full_sync_snapshot(&state.db, &user, &account_id).await {
        Ok(snapshot) => serde_json::to_value(snapshot).map_or_else(
            |_| StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            |mut value| {
                decimal_integers(&mut value);
                Json(value).into_response()
            },
        ),
        Err(db::DbError::Protocol(message)) if message == "Invalid sync account identity" => {
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(error) => ApplicationError::from(error).into_response(),
    }
}

fn decimal_integers(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => items.iter_mut().for_each(decimal_integers),
        serde_json::Value::Object(fields) => fields.values_mut().for_each(decimal_integers),
        serde_json::Value::Number(number) => {
            if let Some(number) = number.as_i64() {
                *value = serde_json::Value::String(number.to_string());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "sync_snapshot_tests.rs"]
mod tests;
