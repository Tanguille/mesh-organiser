//! The hub authenticates every original author before admitting a whole operation.

use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, EntityKind},
};

use super::{author_context, invalid, storage};

pub fn encode_operation(bundle: &super::OperationBundle) -> Result<serde_json::Value, DbError> {
    let envelopes = bundle
        .envelopes
        .iter()
        .map(|event| causal_envelope::canonical_causal_envelope(event).map_err(|_| invalid()))
        .collect::<Result<Vec<_>, _>>()?;

    let mut value = serde_json::json!({"kind":"group_operation","operation_id":bundle.operation_id,"focus_birth_id":bundle.focus_birth_id,"envelopes":envelopes});
    if let Some(context) = &bundle.author_context {
        value["author_context"] = serde_json::Value::String(context.clone());
    }

    Ok(value)
}

fn validate_operation(focus: &str, events: &[CausalEnvelope]) -> Result<(), DbError> {
    let first = events.first().ok_or_else(invalid)?;
    let target = match &first.action {
        Action::GroupRenamed { .. } | Action::GroupNameResolved { .. }
            if events.len() == 1
                && first.entity.kind == EntityKind::Group
                && first.entity.birth_id.as_deref() == Some(focus) =>
        {
            return Ok(());
        }
        Action::ModelMoved { from, to, .. } => {
            if to.is_none()
                && from.as_ref().and_then(|entity| entity.birth_id.as_deref()) != Some(focus)
            {
                return Err(invalid());
            }

            to
        }
        Action::ModelDestinationResolved { to, .. } => to,
        // Deletion/restoration need the author's complete observed membership
        // context before a receiver can prove that no detach/restore was omitted.
        _ => return Err(crate::sync_transport::failure("unsupported")),
    };
    if first.entity.kind != EntityKind::Model {
        return Err(invalid());
    }
    let Some(target) = target else {
        return if events.len() == 1 {
            Ok(())
        } else {
            Err(invalid())
        };
    };
    let [movement, addition] = events else {
        return Err(invalid());
    };
    if target.kind != EntityKind::Group
        || target.birth_id.as_deref() != Some(focus)
        || addition.entity != *target
        || movement.event == addition.event
        || !matches!(&addition.action, Action::GroupMemberAdded { model, membership_event }
            if model == &movement.entity && membership_event == &movement.event)
        || addition.dependencies.len() != 1
        || addition.dependencies[0].entity != movement.entity
        || addition.dependencies[0].event != movement.event
    {
        return Err(invalid());
    }

    Ok(())
}

pub async fn apply(
    db: &mut sqlx::SqliteConnection,
    user: &crate::model::user::User,
    operation: &str,
    focus: &str,
    envelopes: Vec<CausalEnvelope>,
    author_context: Option<&str>,
) -> Result<(), DbError> {
    let context = author_context
        .map(|wire| author_context::parse(wire, &envelopes))
        .transpose()?;
    if context.is_none() {
        validate_operation(focus, &envelopes)?;
    }
    let (mut scopes, prior) = super::checked(db, user).await?;
    if scopes.iter().any(|scope| scope.paused) {
        return Err(crate::sync_transport::failure("scope_paused"));
    }
    let mut missing = prior.missing(&envelopes)?;
    if let Some(context) = &context {
        for dependency in prior.missing_context(&context.frontier)? {
            if !missing.contains(&dependency) {
                missing.push(dependency);
            }
        }
    }
    if !missing.is_empty() {
        let encoded = serde_json::to_string(&missing).map_err(|_| invalid())?;
        if missing.len() > causal_envelope::MAX_CONTEXT_ITEMS
            || encoded.len() > crate::sync_transport::MAX_WIRE_ITEM_BYTES
        {
            return Err(crate::sync_transport::failure("too_large"));
        }

        return Err(crate::sync_transport::failure(&format!(
            "needs_dependencies:{encoded}"
        )));
    }
    if !scopes.iter().any(|scope| {
        scope.entity.kind == EntityKind::Group && scope.entity.birth_id.as_deref() == Some(focus)
    }) {
        return Err(invalid());
    }
    if let Some(context) = &context {
        author_context::validate(db, user, &scopes, &prior, focus, &envelopes, context).await?;
    }
    let mut events = storage::events(db, user).await?;
    events.extend(envelopes.clone());
    let graph = super::graph::Graph::new(&scopes, events)?;
    storage::write(db, user, operation, focus, &envelopes, author_context, true).await?;
    super::project::apply(db, user, &mut scopes, &graph).await?;

    Ok(())
}
