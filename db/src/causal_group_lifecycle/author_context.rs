//! Captured causal cuts, not reconstructed claims about historical observations.
use super::{graph::Graph, invalid, storage::Scope};
use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, Dependency, Dot, EntityKind},
    causal_store,
    model::user::User,
};
use sqlx::SqliteConnection;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Context {
    version: u32,
    pub frontier: Vec<Dependency>,
    events: Vec<Dot>,
}

pub(super) fn capture(
    scopes: &[Scope],
    graph: &Graph,
    events: &[CausalEnvelope],
) -> Result<Option<String>, DbError> {
    if !events.first().is_some_and(|event| {
        matches!(
            event.action,
            Action::GroupDeleted { .. } | Action::GroupRestored { .. }
        )
    }) {
        return Ok(None);
    }
    let mut frontier = graph.relevant_frontier(scopes, &events[0].entity)?;
    frontier.sort_by_key(|dependency| {
        (
            dependency.entity.global_id.clone(),
            dependency.entity.birth_id.clone(),
            dependency.event.writer_id.clone(),
            dependency.event.counter.clone(),
        )
    });
    let context = Context {
        version: 1,
        frontier,
        events: events.iter().map(|event| event.event.clone()).collect(),
    };

    serde_json::to_string(&context)
        .map(Some)
        .map_err(|_| invalid())
}

pub(super) fn parse(wire: &str, events: &[CausalEnvelope]) -> Result<Context, DbError> {
    if wire.len() > crate::sync_transport::MAX_WIRE_ITEM_BYTES {
        return Err(invalid());
    }
    let context: Context = serde_json::from_str(wire).map_err(|_| invalid())?;
    if context.version != 1
        || context.frontier.is_empty()
        || context.frontier.len() > causal_envelope::MAX_CONTEXT_ITEMS
        || context.events
            != events
                .iter()
                .map(|event| event.event.clone())
                .collect::<Vec<_>>()
        || serde_json::to_string(&context).map_err(|_| invalid())? != wire
    {
        return Err(invalid());
    }

    Ok(context)
}

pub(super) async fn fits(
    db: &mut SqliteConnection,
    user: &User,
    events: &[CausalEnvelope],
    context: Option<&str>,
) -> Result<bool, DbError> {
    let (stored_bytes, stored_edges): (i64, i64) = sqlx::query_as("SELECT COALESCE(SUM(length(CAST(author_context AS BLOB))),0),COALESCE(SUM(json_array_length(author_context,'$.frontier')+json_array_length(author_context,'$.events')),0) FROM group_lifecycle_operations WHERE user_id=?")
        .bind(user.id).fetch_one(db).await?;
    let mut bytes = usize::try_from(stored_bytes).map_err(|_| invalid())?;
    let mut edges = usize::try_from(stored_edges).map_err(|_| invalid())?;
    for event in events {
        bytes = bytes.saturating_add(
            causal_envelope::canonical_causal_envelope(event)
                .map_err(|_| invalid())?
                .len(),
        );
        edges = edges
            .saturating_add(event.parents.len())
            .saturating_add(event.dependencies.len());
    }
    if let Some(wire) = context {
        let context: Context = serde_json::from_str(wire).map_err(|_| invalid())?;
        if context.frontier.len() > causal_envelope::MAX_CONTEXT_ITEMS
            || wire.len() > crate::sync_transport::MAX_WIRE_ITEM_BYTES
        {
            return Ok(false);
        }
        bytes = bytes.saturating_add(wire.len());
        edges = edges
            .saturating_add(context.frontier.len())
            .saturating_add(context.events.len());
    }

    Ok(bytes <= causal_store::MAX_OWNER_BYTES && edges <= causal_store::MAX_OWNER_DEPENDENCIES)
}

fn dependency(event: &CausalEnvelope, entity: &causal_envelope::Entity, dot: &Dot) -> bool {
    event
        .dependencies
        .iter()
        .any(|dependency| dependency.entity == *entity && dependency.event == *dot)
}

pub(super) async fn validate(
    db: &mut SqliteConnection,
    user: &User,
    scopes: &[Scope],
    prior: &Graph,
    focus: &str,
    events: &[CausalEnvelope],
    context: &Context,
) -> Result<(), DbError> {
    let (scopes, cut) = prior.authored_cut(scopes, &context.frontier)?;
    let first = events.first().ok_or_else(invalid)?;
    if first.entity.kind != EntityKind::Group
        || first.entity.birth_id.as_deref() != Some(focus)
        || first.parents != cut.frontier(&first.entity, None)
    {
        return Err(invalid());
    }
    match &first.action {
        Action::GroupDeleted { .. } => validate_delete(&scopes, &cut, events),
        Action::GroupRestored { deletion } => {
            let previous = deletion_members(db, user, deletion).await?;
            if previous.iter().any(|event| !cut.events.contains(event)) {
                return Err(invalid());
            }
            validate_restore(&scopes, &cut, events, &previous, deletion)
        }
        _ => Err(invalid()),
    }
}

fn validate_delete(
    scopes: &[Scope],
    cut: &Graph,
    events: &[CausalEnvelope],
) -> Result<(), DbError> {
    let deletion = &events[0];
    let members: Vec<_> = scopes
        .iter()
        .filter(|scope| {
            scope.entity.kind == EntityKind::Model
                && cut
                    .moves(&scope.entity, None)
                    .iter()
                    .any(|choice| choice.destination.as_ref() == Some(&deletion.entity))
        })
        .collect();
    if !deletion.dependencies.is_empty() || events.len() != members.len() + 1 {
        return Err(invalid());
    }
    for member in members {
        let matches: Vec<_> = events[1..]
            .iter()
            .filter(|event| event.entity == member.entity)
            .collect();
        let [detached] = matches.as_slice() else {
            return Err(invalid());
        };
        if !matches!(&detached.action, Action::ModelDetached { group, deletion: dot } if group == &deletion.entity && dot == &deletion.event)
            || detached.parents != cut.frontier(&member.entity, None)
            || detached.dependencies.len() != 1
            || !dependency(detached, &deletion.entity, &deletion.event)
        {
            return Err(invalid());
        }
    }

    Ok(())
}

async fn deletion_members(
    db: &mut SqliteConnection,
    user: &User,
    deletion: &Dot,
) -> Result<Vec<CausalEnvelope>, DbError> {
    // A restore observes the entire already-admitted deletion operation, not
    // merely its group event. Historical unmanifested deletions remain gated.
    let (operation, count, context): (String, i64, Option<String>) = sqlx::query_as("SELECT o.operation_id,o.event_count,o.author_context FROM group_lifecycle_events e JOIN group_lifecycle_operations o USING(user_id,operation_id) WHERE e.user_id=? AND e.writer_id=? AND e.counter=?")
        .bind(user.id).bind(&deletion.writer_id).bind(super::counter(deletion)?).fetch_optional(&mut *db).await?.ok_or_else(invalid)?;
    let wires: Vec<String> = sqlx::query_scalar("SELECT c.envelope FROM group_lifecycle_events e JOIN causal_events c USING(user_id,writer_id,counter) WHERE e.user_id=? AND e.operation_id=? ORDER BY position")
        .bind(user.id).bind(operation).fetch_all(db).await?;
    let events = wires
        .iter()
        .map(|wire| causal_envelope::parse_causal_envelope(wire).map_err(|_| invalid()))
        .collect::<Result<Vec<_>, _>>()?;
    if i64::try_from(events.len()).map_err(|_| invalid())? != count
        || !events.first().is_some_and(|event| {
            event.event == *deletion && matches!(event.action, Action::GroupDeleted { .. })
        })
    {
        return Err(invalid());
    }
    parse(context.as_deref().ok_or_else(invalid)?, &events)?;

    Ok(events)
}

fn validate_restore(
    scopes: &[Scope],
    cut: &Graph,
    events: &[CausalEnvelope],
    previous: &[CausalEnvelope],
    deletion: &Dot,
) -> Result<(), DbError> {
    let restoration = &events[0];
    let members: Vec<_> = scopes.iter().filter(|scope| scope.entity.kind == EntityKind::Model
        && previous.iter().any(|event| event.entity == scope.entity && matches!(&event.action, Action::ModelDetached { group, deletion: dot } if group == &restoration.entity && dot == deletion))
        && { let choices = cut.moves(&scope.entity, None); choices.len() == 1 && choices[0].destination.is_none() }).collect();
    if restoration.dependencies.len() != 1
        || !dependency(restoration, &restoration.entity, deletion)
        || events.len() != 1 + 2 * members.len()
    {
        return Err(invalid());
    }
    let mut remaining = members;
    let mut parent = &restoration.event;
    for pair in events[1..].as_chunks::<2>().0 {
        let restored = &pair[0];
        let added = &pair[1];
        let position = remaining
            .iter()
            .position(|scope| scope.entity == restored.entity)
            .ok_or_else(invalid)?;
        remaining.remove(position);
        if !matches!(&restored.action, Action::ModelRestored { group, deletion: dot, restoration: root } if group == &restoration.entity && dot == deletion && root == &restoration.event)
            || restored.parents != cut.frontier(&restored.entity, None)
            || restored.dependencies.len() != 2
            || !dependency(restored, &restoration.entity, deletion)
            || !dependency(restored, &restoration.entity, &restoration.event)
            || added.entity != restoration.entity
            || added.parents != [parent.clone()]
            || !matches!(&added.action, Action::GroupMemberAdded { model, membership_event } if model == &restored.entity && membership_event == &restored.event)
            || added.dependencies.len() != 1
            || !dependency(added, &restored.entity, &restored.event)
        {
            return Err(invalid());
        }
        parent = &added.event;
    }

    Ok(())
}
