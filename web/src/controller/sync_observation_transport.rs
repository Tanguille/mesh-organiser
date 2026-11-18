//! Test-router-only physical transfer. Authentication is rechecked at commit.
use axum::{
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};

use db::sync_transport::observations;
use service::{
    AppState, ServiceError,
    sync_observation_service::{self, IncomingObservation},
};

use crate::user::{AuthSession, CurrentSyncUser};

use super::{current, response, secret};

fn failure(error: ServiceError) -> Response {
    match error {
        ServiceError::DatabaseError(error) => response::<serde_json::Value>(Err(error)),
        error => crate::error::ApplicationError::from(error).into_response(),
    }
}
pub(super) async fn publish(
    CurrentSyncUser(user, account): CurrentSyncUser,
    session: AuthSession,
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let body =
        match tokio::time::timeout(std::time::Duration::from_secs(30), metadata(&mut multipart))
            .await
        {
            Ok(Ok(body)) => body,
            Ok(Err(error)) => return error,
            Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
        };
    if !current(&session, body.protocol).await {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    }
    let credential = secret(&headers).to_owned();
    let plan = match observations::inspect(
        &state.db,
        &user,
        &account,
        &body.connection_id,
        &credential,
        body.writer_nonce.as_deref(),
        body.item.clone(),
    )
    .await
    {
        Ok(plan) => plan,
        Err(error) => return response::<serde_json::Value>(Err(error)),
    };
    let deadline = receive_deadline(plan.contents());
    // Client-paced reads must never hold the ordinary import/GC lock.
    let mut incoming = match IncomingObservation::new(&state, &plan).await {
        Ok(value) => value,
        Err(error) => return failure(error),
    };
    // Any timeout or error returns here, dropping `incoming`, which clears its stage via `Stage::drop`.
    match tokio::time::timeout(deadline, receive(&mut multipart, &mut incoming)).await {
        Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
        Ok(Err(error)) => return error,
        Ok(Ok(())) => {}
    }
    // Losing the HTTP future cannot cancel the file/DB commit sequence. Exact retries
    // recover an ACK after a disconnect, without creating another group or file.
    match tokio::spawn(async move {
        sync_observation_service::install_received(
            state,
            user,
            account,
            body.connection_id,
            credential,
            body.writer_nonce,
            body.item,
            incoming,
        )
        .await
    })
    .await
    {
        Ok(Ok(published)) => response(Ok(published)),
        Ok(Err(error)) => failure(error),
        Err(error) => failure(error.into()),
    }
}
#[allow(clippy::result_large_err)] // Local HTTP helpers return the already-built rejection response.
async fn metadata(multipart: &mut Multipart) -> Result<super::Publish, Response> {
    let mut field = match multipart.next_field().await {
        Ok(Some(field)) if field.name() == Some("request") => field,
        Ok(_) => return Err(StatusCode::CONFLICT.into_response()),
        Err(error) => return Err(error.status().into_response()),
    };
    let mut bytes = Vec::new();
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|error| error.status().into_response())?
    {
        // Only the small JSON `request` field is bounded here; blobs stream to disk and the
        // route-level body limit (sized for MAX_BLOB_BYTES + MAX_TOTAL_BYTES) covers the rest.
        if bytes.len().saturating_add(chunk.len()) > db::sync_transport::MAX_WIRE_ITEM_BYTES {
            return Err(StatusCode::PAYLOAD_TOO_LARGE.into_response());
        }
        bytes.extend_from_slice(&chunk);
    }

    serde_json::from_slice(&bytes).map_err(|_| StatusCode::CONFLICT.into_response())
}
/// Idle (not total) deadline: a progressing transfer of up to `MAX_TOTAL_BYTES` must not be cut
/// off on a slow link, while a stalled peer still releases its staging permit.
const RECEIVE_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Slow-link floor for the total receive deadline. The idle timeout alone lets a peer that
/// trickles one chunk per interval hold a staging slot indefinitely.
const RECEIVE_MIN_BYTES_PER_SECOND: u64 = 16 * 1024;

/// Total deadline for one upload: the idle allowance plus the time the declared bytes take at
/// the slow-link floor. Scales with the plan instead of a fixed cap.
fn receive_deadline(contents: &[observations::BlobContent]) -> std::time::Duration {
    let declared_bytes = contents.iter().fold(0_u64, |total, content| {
        total.saturating_add(u64::try_from(content.size).unwrap_or(0))
    });

    RECEIVE_IDLE_TIMEOUT.saturating_add(std::time::Duration::from_secs(
        declared_bytes / RECEIVE_MIN_BYTES_PER_SECOND,
    ))
}

#[allow(clippy::result_large_err)] // Local HTTP helpers return the already-built rejection response.
async fn idle<T>(
    step: impl Future<Output = Result<T, axum::extract::multipart::MultipartError>>,
) -> Result<T, Response> {
    tokio::time::timeout(RECEIVE_IDLE_TIMEOUT, step)
        .await
        .map_or_else(
            |_| Err(StatusCode::REQUEST_TIMEOUT.into_response()),
            |result| result.map_err(|error| error.status().into_response()),
        )
}
#[allow(clippy::result_large_err)] // Preserve multipart status and structured protocol errors without wrappers.
async fn receive(
    multipart: &mut Multipart,
    incoming: &mut IncomingObservation,
) -> Result<(), Response> {
    while let Some(mut field) = idle(multipart.next_field()).await? {
        let key = field
            .name()
            .and_then(|name| name.strip_prefix("blob."))
            .ok_or_else(|| StatusCode::CONFLICT.into_response())?;
        let mut blob = incoming.start_blob(key).await.map_err(failure)?;
        while let Some(chunk) = idle(field.chunk()).await? {
            blob.write(&chunk).await.map_err(failure)?;
        }
        incoming.finish_blob(blob).await.map_err(failure)?;
    }
    incoming.complete().map_err(failure)
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Download {
    protocol: u32,
    connection_id: String,
}
pub(super) async fn download(
    CurrentSyncUser(user, account): CurrentSyncUser,
    session: AuthSession,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((sequence, key)): Path<(String, String)>,
    Query(query): Query<Download>,
) -> Response {
    if !current(&session, query.protocol).await {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    }
    let Some(sequence) = super::cursor(&sequence) else {
        return StatusCode::CONFLICT.into_response();
    };
    let (content, blob) = match observations::download_blob(
        &state.db,
        &user,
        &account,
        &query.connection_id,
        secret(&headers),
        sequence,
        &key,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => return response::<serde_json::Value>(Err(error)),
    };
    let verified = match sync_observation_service::prepare_download(&state, &content, &blob).await {
        Ok(value) => value,
        Err(error) => return failure(error),
    };
    let size = verified.size;
    let stream = tokio_util::io::ReaderStream::new(verified);

    (
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (header::CONTENT_LENGTH, size.to_string()),
        ],
        axum::body::Body::from_stream(stream),
    )
        .into_response()
}
