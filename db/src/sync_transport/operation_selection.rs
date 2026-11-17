//! Recover eligible original journal operations without making local edits depend on queue capacity.
use super::{failure, outbox, pull::Authorization};
use crate::{
    DbError,
    causal_envelope::{CausalEnvelope, Dot},
    causal_group_lifecycle,
    db_context::DbContext,
    model::user::User,
};
use sqlx::SqliteConnection;

#[derive(Debug)]
pub enum OperationSelection {
    Queued {
        operation_id: String,
        outbox_id: String,
        event: Dot,
    },
    /// These counts cover the local combined-operation journal, not all account changes.
    NoEligible {
        untracked_operations: usize,
        other_link_operations: usize,
        discarded_operations: usize,
    },
}

pub(crate) async fn record_local(
    db: &mut SqliteConnection,
    user: &User,
    operation: &str,
    first: &CausalEnvelope,
) -> Result<(), DbError> {
    let Some(authorization) =
        Authorization::recording_key(db, user, &first.event.writer_id).await?
    else {
        return Ok(());
    };
    sqlx::query("INSERT INTO sync_local_operation_selections(user_id,operation_id,writer_id,counter,authorization) VALUES(?,?,?,?,?)")
        .bind(user.id).bind(operation).bind(&first.event.writer_id).bind(first.event.counter.parse::<i64>().map_err(|_| failure("unsupported"))?).bind(authorization).execute(db).await?;

    Ok(())
}

/// Every operation enqueue uses this disposition, including explicit name choices.
pub(crate) async fn selected_in(
    db: &mut SqliteConnection,
    auth: &Authorization,
    operation: &str,
    outbox_id: &str,
) -> Result<(), DbError> {
    let row: Option<(String,Option<String>)> = sqlx::query_as("SELECT authorization,outbox_id FROM sync_local_operation_selections WHERE user_id=? AND operation_id=?")
        .bind(auth.user.id).bind(operation).fetch_optional(&mut *db).await?;
    let Some((authorization, selected)) = row else {
        // Older operations remain explicit-only; absence is not evidence of an
        // unselected operation and automatic selection never includes them.
        return Ok(());
    };
    if authorization != auth.identity_choice_key() {
        return Err(failure("stale_link"));
    }
    if let Some(selected) = selected {
        if selected != outbox_id {
            return Err(failure("publication_discarded"));
        }
    } else {
        sqlx::query("UPDATE sync_local_operation_selections SET outbox_id=? WHERE user_id=? AND operation_id=? AND outbox_id IS NULL")
            .bind(outbox_id).bind(auth.user.id).bind(operation).execute(db).await?;
    }

    Ok(())
}

pub async fn select_next(
    db: &DbContext,
    auth: &Authorization,
) -> Result<OperationSelection, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    auth.validate(&mut tx).await?;
    let row: Option<(String,String,i64)> = sqlx::query_as("SELECT operation_id,writer_id,counter FROM sync_local_operation_selections WHERE user_id=? AND authorization=? AND outbox_id IS NULL ORDER BY selection_id LIMIT 1")
        .bind(auth.user.id).bind(auth.identity_choice_key()).fetch_optional(&mut *tx).await?;
    let Some((operation_id, writer_id, counter)) = row else {
        let result = no_eligible(&mut tx, auth).await?;
        tx.commit().await?;

        return Ok(result);
    };
    // Ordinary edits have already committed. A full queue or failed export only
    // rolls back this selection; it cannot discard or partially rewrite the edit.
    if !causal_group_lifecycle::publication_ready(&mut tx, &auth.user).await? {
        tx.commit().await?;

        return Err(failure("untracked_changes"));
    }
    let event = Dot {
        writer_id,
        counter: counter.to_string(),
    };
    let bundle =
        causal_group_lifecycle::export_operation_in(&mut tx, &auth.user, &auth.identity, &event)
            .await?;
    let item = super::encode_operation(&bundle)?;
    if item["operation_id"].as_str() != Some(operation_id.as_str()) {
        return Err(failure("identity_blocked"));
    }
    let outbox_id = outbox::queue_in(&mut tx, auth, &event, &item).await?;
    tx.commit().await?;

    Ok(OperationSelection::Queued {
        operation_id,
        outbox_id,
        event,
    })
}

async fn no_eligible(
    db: &mut SqliteConnection,
    auth: &Authorization,
) -> Result<OperationSelection, DbError> {
    let (untracked, other, discarded): (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM group_lifecycle_events e WHERE e.user_id=?1 AND e.position=0 AND NOT EXISTS(SELECT 1 FROM causal_event_receipts r WHERE r.user_id=e.user_id AND r.writer_id=e.writer_id AND r.counter=e.counter) AND NOT EXISTS(SELECT 1 FROM sync_local_operation_selections s WHERE s.user_id=e.user_id AND s.operation_id=e.operation_id)),(SELECT COUNT(*) FROM sync_local_operation_selections WHERE user_id=?1 AND authorization<>?2 AND outbox_id IS NULL),(SELECT COUNT(*) FROM sync_local_operation_selections s WHERE s.user_id=?1 AND s.outbox_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM sync_publication_outbox o WHERE o.outbox_id=s.outbox_id AND o.user_id=s.user_id))")
        .bind(auth.user.id).bind(auth.identity_choice_key()).fetch_one(db).await?;

    Ok(OperationSelection::NoEligible {
        untracked_operations: usize::try_from(untracked).map_err(|_| failure("too_large"))?,
        other_link_operations: usize::try_from(other).map_err(|_| failure("too_large"))?,
        discarded_operations: usize::try_from(discarded).map_err(|_| failure("too_large"))?,
    })
}
