//! Scoped portable group names. Network sync remains disabled.

use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, Dot},
    db_context::DbContext,
    model::user::User,
    sync_peer::{validate_identity, validate_local},
    user_db::SyncIdentity,
};

mod graph;
mod journal;
#[cfg(test)]
#[path = "causal_group_rename/name_choice_tests.rs"]
mod name_choice_tests;
mod storage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameToken {
    pub entity: causal_envelope::Entity,
    pub generation: i64,
    pub revision: i64,
    pub frontier: Vec<Dot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameChoice {
    pub name: String,
    pub events: Vec<Dot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameState {
    Unavailable,
    Ready {
        name: String,
        token: NameToken,
    },
    Pending {
        token: NameToken,
        missing: Vec<Dot>,
    },
    Conflict {
        choices: Vec<NameChoice>,
        token: NameToken,
    },
    Paused {
        name: String,
        choices: Vec<NameChoice>,
        token: NameToken,
        missing: Vec<Dot>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveResult {
    Resolved(NameState),
    Stale(NameState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameImportResult {
    Applied(NameState),
    /// No receipt was admitted. The sender must retain the event and provide
    /// these dependencies before retrying; this response is not an acknowledgement.
    NeedsDependencies {
        missing: Vec<Dot>,
        state: NameState,
    },
}

/// Only checked source database operations may construct a transferable event.
/// Caller-provided peer pins do not authenticate a network sender.
#[derive(Debug)]
pub struct RenameBundle {
    envelope: CausalEnvelope,
    source_identity: SyncIdentity,
    root: Dot,
    wire: String,
}

impl RenameBundle {
    #[must_use]
    pub const fn envelope(&self) -> &CausalEnvelope {
        &self.envelope
    }

    #[must_use]
    pub const fn source_identity(&self) -> &SyncIdentity {
        &self.source_identity
    }
}

fn invalid() -> DbError {
    DbError::Protocol("Invalid or unsupported portable group name history".into())
}

fn counter(dot: &Dot) -> Result<i64, DbError> {
    dot.counter
        .parse::<i64>()
        .ok()
        .filter(|counter| *counter > 0)
        .ok_or_else(invalid)
}

async fn inspect(
    db: &mut sqlx::SqliteConnection,
    scope: &mut storage::Scope,
) -> Result<graph::Graph, DbError> {
    let graph = graph::load(db, scope).await?;
    // Earlier scoped staging could retain children without their ancestors. Their
    // unknown closure cannot be reserved within a finite budget, so keep that
    // evidence visible while requiring an explicit checkpoint before resuming.
    if !scope.paused && graph.has_missing() {
        storage::pause(db, scope).await?;
    }

    Ok(graph)
}

pub async fn get_state(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    group_id: i64,
) -> Result<NameState, DbError> {
    validate_identity(identity)?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut tx, user, identity).await?;
    if let Some(state) = crate::causal_group_lifecycle::name_state(&mut tx, user, group_id).await? {
        tx.commit().await?;

        return Ok(state);
    }
    let state = if let Some(mut scope) = storage::load(&mut tx, user, group_id).await? {
        inspect(&mut tx, &mut scope).await?.state(&scope)
    } else {
        NameState::Unavailable
    };
    tx.commit().await?;

    Ok(state)
}

pub async fn export_event(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    event: &Dot,
) -> Result<RenameBundle, DbError> {
    validate_identity(identity)?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut tx, user, identity).await?;
    let (birth, wire): (String, String) = sqlx::query_as("SELECT n.birth_id, e.envelope FROM group_rename_events n JOIN causal_events e ON e.user_id = n.user_id AND e.writer_id = n.writer_id AND e.counter = n.counter WHERE n.user_id = ? AND n.writer_id = ? AND n.counter = ? AND e.encoding = 'portable_v2'")
        .bind(user.id).bind(&event.writer_id).bind(counter(event)?).fetch_optional(&mut *tx).await?.ok_or_else(invalid)?;
    let id = storage::local_id(&mut tx, user, &birth).await?;
    let mut scope = storage::load(&mut tx, user, id)
        .await?
        .ok_or_else(invalid)?;
    inspect(&mut tx, &mut scope).await?;
    if scope.paused {
        tx.commit().await?;

        return Err(invalid());
    }
    let envelope = causal_envelope::parse_causal_envelope(&wire).map_err(|_| invalid())?;
    if envelope.event != *event {
        return Err(invalid());
    }
    tx.commit().await?;

    Ok(RenameBundle {
        envelope,
        source_identity: identity.clone(),
        root: scope.root.event,
        wire,
    })
}

pub async fn import_event(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    peer: &crate::sync_peer::PeerBinding,
    bundle: &RenameBundle,
) -> Result<RenameImportResult, DbError> {
    validate_identity(identity)?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut tx, user, identity).await?;
    if peer.local != *identity || peer.remote != bundle.source_identity {
        return Err(invalid());
    }
    let pinned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_peer_bindings WHERE user_id = ? AND local_instance_id = ? AND local_account_id = ? AND remote_instance_id = ? AND remote_account_id = ?)")
        .bind(user.id).bind(&identity.instance_id).bind(&identity.account_id).bind(&peer.remote.instance_id).bind(&peer.remote.account_id).fetch_one(&mut *tx).await?;
    if !pinned {
        return Err(invalid());
    }
    let birth = bundle
        .envelope
        .entity
        .birth_id
        .as_deref()
        .ok_or_else(invalid)?;
    let id = storage::local_id(&mut tx, user, birth).await?;
    let mut scope = storage::load(&mut tx, user, id)
        .await?
        .ok_or_else(invalid)?;
    let previous_graph = inspect(&mut tx, &mut scope).await?;
    if scope.paused {
        tx.commit().await?;

        return Err(invalid());
    }
    if scope.root.event != bundle.root || scope.root.entity != bundle.envelope.entity {
        return Err(invalid());
    }
    if storage::has_event(&mut tx, &scope, &bundle.envelope.event).await? {
        let existing: String = sqlx::query_scalar("SELECT envelope FROM causal_events WHERE user_id = ? AND writer_id = ? AND counter = ?")
            .bind(user.id).bind(&bundle.envelope.event.writer_id).bind(counter(&bundle.envelope.event)?).fetch_one(&mut *tx).await?;
        if existing != bundle.wire {
            return Err(invalid());
        }
        let state = previous_graph.state(&scope);
        tx.commit().await?;

        return Ok(RenameImportResult::Applied(state));
    }
    let missing = previous_graph.request_parents(&bundle.envelope.parents);
    if !missing.is_empty() {
        let state = previous_graph.state(&scope);
        tx.commit().await?;

        return Ok(RenameImportResult::NeedsDependencies { missing, state });
    }
    journal::remote(&mut tx, &scope, &bundle.envelope, &bundle.wire).await?;
    let graph = graph::load(&mut tx, &scope).await?;
    // Every accepted frontier must still fit one explicit resolution envelope.
    // Reject the admission atomically rather than dropping an unseen branch.
    if graph.potential_tips > causal_envelope::MAX_CONTEXT_ITEMS {
        return Err(invalid());
    }
    storage::project(&mut tx, &mut scope, &graph, &crate::time_now()).await?;
    let state = graph.state(&scope);
    tx.commit().await?;

    Ok(RenameImportResult::Applied(state))
}

pub async fn resolve(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    group_id: i64,
    token: &NameToken,
    name: &str,
) -> Result<ResolveResult, DbError> {
    validate_identity(identity)?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut tx, user, identity).await?;
    if let Some(state) =
        crate::causal_group_lifecycle::resolve_name(&mut tx, user, group_id, token, name).await?
    {
        tx.commit().await?;

        return Ok(state);
    }
    let Some(mut scope) = storage::load(&mut tx, user, group_id).await? else {
        tx.commit().await?;

        return Ok(ResolveResult::Stale(NameState::Unavailable));
    };
    let state = inspect(&mut tx, &mut scope).await?.state(&scope);
    let NameState::Conflict {
        token: current,
        choices,
    } = &state
    else {
        tx.commit().await?;

        return Ok(ResolveResult::Stale(state));
    };
    if current != token {
        tx.commit().await?;

        return Ok(ResolveResult::Stale(state));
    }
    if !choices.iter().any(|choice| choice.name == name) {
        return Err(DbError::Protocol(
            "Selected name is not an offered choice".into(),
        ));
    }
    let recorded = journal::local(
        &mut tx,
        &scope,
        current.frontier.clone(),
        Action::GroupNameResolved {
            name: name.to_owned(),
        },
    )
    .await?;
    if !recorded {
        storage::pause(&mut tx, &mut scope).await?;
        let state = graph::load(&mut tx, &scope).await?.state(&scope);
        tx.commit().await?;

        return Ok(ResolveResult::Stale(state));
    }
    let graph = graph::load(&mut tx, &scope).await?;
    storage::project(&mut tx, &mut scope, &graph, &crate::time_now()).await?;
    let state = graph.state(&scope);
    tx.commit().await?;

    Ok(ResolveResult::Resolved(state))
}

/// Runs inside the real `edit_group` transaction. Unbound or invalidated groups
/// retain their existing local-only history; a conflict never becomes a rename.
pub(crate) async fn try_edit(
    db: &mut sqlx::SqliteConnection,
    user: &User,
    group_id: i64,
    name: &str,
    timestamp: &str,
    rekey: bool,
) -> Result<bool, DbError> {
    if rekey {
        storage::invalidate(db, user, group_id).await?;

        return Ok(false);
    }
    let Some(mut scope) = storage::load(db, user, group_id).await? else {
        storage::invalidate(db, user, group_id).await?;

        return Ok(false);
    };
    if scope.paused {
        if scope.current_name != name {
            storage::pause(db, &mut scope).await?;
        }

        return Ok(false);
    }
    let state = inspect(db, &mut scope).await?.state(&scope);
    if scope.paused {
        return Ok(false);
    }
    if scope.current_name != name && !journal::capacity(db, &scope, 0, 0).await? {
        // Capacity is a sync limitation, not permission to discard local edits.
        // Keep all portable choices and continue through the local-only journal.
        storage::pause(db, &mut scope).await?;

        return Ok(false);
    }
    let NameState::Ready {
        name: current,
        token,
    } = state
    else {
        return Err(DbError::Protocol(
            "Group name choice or complete ancestry required".into(),
        ));
    };
    if current == name {
        sqlx::query("UPDATE models_group SET group_last_modified = ? WHERE group_id = ? AND group_user_id = ?")
            .bind(timestamp).bind(group_id).bind(user.id).execute(db).await?;

        return Ok(true);
    }
    let recorded = journal::local(
        db,
        &scope,
        token.frontier,
        Action::GroupRenamed {
            name: name.to_owned(),
        },
    )
    .await?;
    if !recorded {
        storage::pause(db, &mut scope).await?;

        return Ok(false);
    }
    let graph = graph::load(db, &scope).await?;
    storage::project(db, &mut scope, &graph, timestamp).await?;

    Ok(true)
}
