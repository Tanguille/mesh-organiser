use sqlx::SqliteConnection;

use super::{
    invalid,
    storage::{self, Scope},
};
use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, Dependency, Dot},
    causal_store,
};

pub(super) async fn local(
    db: &mut SqliteConnection,
    scope: &Scope,
    parents: Vec<Dot>,
    dependencies: Vec<Dependency>,
    action: Action,
) -> Result<bool, DbError> {
    if parents.len() > causal_envelope::MAX_CONTEXT_ITEMS
        || dependencies.len() > causal_envelope::MAX_CONTEXT_ITEMS
    {
        return Ok(false);
    }
    let (writer_id, previous) = crate::causal_clock::read(db, scope.user_id).await?;
    let counter = previous.checked_add(1).ok_or_else(invalid)?;
    let envelope = CausalEnvelope {
        version: 2,
        entity: scope.root.entity.clone(),
        event: Dot {
            writer_id,
            counter: counter.to_string(),
        },
        parents,
        dependencies,
        action,
    };
    let wire = causal_envelope::canonical_causal_envelope(&envelope).map_err(|_| invalid())?;
    if !capacity(
        db,
        scope,
        wire.len(),
        envelope.parents.len() + envelope.dependencies.len(),
    )
    .await?
    {
        return Ok(false);
    }
    crate::causal_clock::set(
        db,
        scope.user_id,
        &envelope.event.writer_id,
        previous,
        counter,
    )
    .await?;
    sqlx::query(
        "INSERT INTO causal_events(user_id, writer_id, counter, envelope) VALUES (?, ?, ?, ?)",
    )
    .bind(scope.user_id)
    .bind(&envelope.event.writer_id)
    .bind(counter)
    .bind(wire)
    .execute(&mut *db)
    .await?;
    storage::metadata(db, scope, &envelope.event, "local").await?;

    Ok(true)
}

pub(super) async fn capacity(
    db: &mut SqliteConnection,
    scope: &Scope,
    bytes: usize,
    edges: usize,
) -> Result<bool, DbError> {
    let (count, stored_bytes, stored_edges): (i64, i64, i64) = sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(length(CAST(e.envelope AS BLOB))), 0), COALESCE(SUM(json_array_length(e.envelope, '$.parents') + json_array_length(e.envelope, '$.dependencies')), 0) FROM model_move_events m JOIN causal_events e ON e.user_id = m.user_id AND e.writer_id = m.writer_id AND e.counter = m.counter WHERE m.user_id = ? AND m.birth_id = ?")
        .bind(scope.user_id).bind(&scope.birth_id).fetch_one(db).await?;

    Ok(
        usize::try_from(count).map_err(|_| invalid())? < causal_store::MAX_OWNER_EVENTS
            && usize::try_from(stored_bytes)
                .map_err(|_| invalid())?
                .saturating_add(scope.root_wire.len())
                .saturating_add(bytes)
                <= causal_store::MAX_OWNER_BYTES
            && usize::try_from(stored_edges)
                .map_err(|_| invalid())?
                .saturating_add(edges)
                <= causal_store::MAX_OWNER_DEPENDENCIES,
    )
}

pub(super) async fn remote(
    db: &mut SqliteConnection,
    scope: &Scope,
    envelope: &CausalEnvelope,
    wire: &str,
) -> Result<(), DbError> {
    causal_store::store_checked_receipt(db, scope.user_id, envelope, wire).await?;
    storage::metadata(db, scope, &envelope.event, "checked_transfer").await
}
