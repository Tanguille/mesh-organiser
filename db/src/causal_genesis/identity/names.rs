//! A name choice and its original publication cannot be separated by a crash.

use sqlx::SqliteConnection;

use crate::causal_group_rename::{NameState, NameToken, ResolveResult};

use super::{Authorization, DbError, NamePublication, choice, invalid};

pub async fn resolve_name_publication(
    db: &super::DbContext,
    auth: &Authorization,
    decision_id: &str,
    token: &NameToken,
    name: &str,
) -> Result<NamePublication, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let result = resolve_in(&mut tx, auth, decision_id, token, name).await?;
    tx.commit().await?;
    Ok(result)
}

#[allow(clippy::too_many_lines)] // The authenticated request, selected event and outbox form one immutable receipt.
async fn resolve_in(
    db: &mut SqliteConnection,
    auth: &Authorization,
    decision_id: &str,
    token: &NameToken,
    name: &str,
) -> Result<NamePublication, DbError> {
    let choice = choice(db, auth, decision_id).await?;
    let (Some(writer), Some(number)) = (choice.writer_id, choice.counter) else {
        return Err(invalid());
    };
    // Only confirmed Same decisions have this independent root. Resolve through
    // its stable birth, since guarded retention may have recreated the local row.
    let birth: String = sqlx::query_scalar("SELECT birth_id FROM group_identity_observations WHERE user_id=? AND observation_id=? AND writer_id=? AND counter=?")
        .bind(auth.user.id).bind(decision_id).bind(writer).bind(number).fetch_optional(&mut *db).await?.ok_or_else(invalid)?;
    let (group_id, state) = current(db, auth, &birth).await?;
    let request = super::encode(
        &serde_json::json!({"entity":token.entity,"generation":token.generation,"revision":token.revision,"frontier":token.frontier,"name":name}),
    )?;
    let request_id = crate::util::sha256_hex(request.as_bytes());
    let prior: Option<(String,String,i64,String)> = sqlx::query_as("SELECT request,writer_id,counter,outbox_id FROM group_identity_name_publications WHERE user_id=? AND decision_id=? AND request_id=?")
        .bind(auth.user.id).bind(decision_id).bind(&request_id).fetch_optional(&mut *db).await?;
    if let Some((original, writer_id, counter, outbox_id)) = prior {
        if original != request {
            return Err(invalid());
        }
        let event = super::Dot {
            writer_id,
            counter: counter.to_string(),
        };
        crate::sync_transport::outbox::require_retained(db, auth, &outbox_id, &event).await?;

        return Ok(NamePublication::Queued {
            outbox_id,
            event,
            already_present: true,
            state,
        });
    }
    let result = resolve_and_queue(db, auth, group_id, state, token, name).await?;
    let NamePublication::Queued {
        outbox_id,
        event,
        state,
        ..
    } = result
    else {
        return Ok(result);
    };
    let (count, bytes):(i64,i64) = sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(length(CAST(request AS BLOB))),0) FROM group_identity_name_publications WHERE user_id=?")
        .bind(auth.user.id).fetch_one(&mut *db).await?;
    if usize::try_from(count).map_err(|_| invalid())? >= crate::causal_store::MAX_OWNER_EVENTS
        || usize::try_from(bytes)
            .map_err(|_| invalid())?
            .saturating_add(request.len())
            > crate::causal_store::MAX_OWNER_BYTES
    {
        return Err(invalid());
    }
    sqlx::query("INSERT INTO group_identity_name_publications(user_id,decision_id,request_id,request,writer_id,counter,outbox_id) VALUES(?,?,?,?,?,?,?)")
        .bind(auth.user.id).bind(decision_id).bind(request_id).bind(request).bind(&event.writer_id).bind(super::super::counter(&event)?).bind(&outbox_id).execute(db).await?;
    Ok(NamePublication::Queued {
        outbox_id,
        event,
        already_present: false,
        state,
    })
}

/// Resolve only inside the caller's write transaction, before recording its exact request.
pub(super) async fn resolve_and_queue(
    db: &mut SqliteConnection,
    auth: &Authorization,
    group_id: Option<i64>,
    state: NameState,
    token: &NameToken,
    name: &str,
) -> Result<NamePublication, DbError> {
    let Some(group_id) = group_id else {
        return Ok(NamePublication::Stale { state });
    };
    let result = crate::causal_group_lifecycle::resolve_name(db, &auth.user, group_id, token, name)
        .await?
        .ok_or_else(invalid)?;
    let state = match result {
        ResolveResult::Resolved(state) => state,
        ResolveResult::Stale(state) => return Ok(NamePublication::Stale { state }),
    };
    let NameState::Ready { token, .. } = &state else {
        return Err(invalid());
    };
    let [event] = token.frontier.as_slice() else {
        return Err(invalid());
    };
    let event = event.clone();
    let bundle =
        crate::causal_group_lifecycle::export_operation_in(db, &auth.user, &auth.identity, &event)
            .await?;
    let item = crate::sync_transport::encode_operation(&bundle)?;
    let outbox_id = crate::sync_transport::outbox::queue_in(db, auth, &event, &item).await?;
    Ok(NamePublication::Queued {
        outbox_id,
        event,
        already_present: false,
        state,
    })
}

pub(super) async fn current(
    db: &mut SqliteConnection,
    auth: &Authorization,
    birth: &str,
) -> Result<(Option<i64>, NameState), DbError> {
    let group_id: Option<i64> = sqlx::query_scalar("SELECT r.local_group_id FROM group_lifecycle_group_rows r JOIN causal_genesis_bindings b ON b.user_id=r.user_id AND b.entity_kind='group' AND b.birth_id=r.birth_id JOIN models_group g ON g.group_id=r.local_group_id AND g.group_user_id=r.user_id AND g.group_birth_id=b.birth_id AND g.group_unique_global_id=b.global_id WHERE r.user_id=? AND r.birth_id=? AND r.state='present' AND g.group_deleted_at IS NULL AND g.group_deletion_operation_id IS NULL")
        .bind(auth.user.id).bind(birth).fetch_optional(&mut *db).await?;
    let state = if let Some(group_id) = group_id {
        crate::causal_group_lifecycle::name_state(db, &auth.user, group_id)
            .await?
            .unwrap_or(NameState::Unavailable)
    } else {
        NameState::Unavailable
    };

    Ok((group_id, state))
}
