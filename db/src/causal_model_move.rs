//! Scoped model destinations. Network sync remains disabled.

mod graph;
mod journal;
mod project;
mod storage;
mod targets;

use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, Dependency, Dot, Entity},
    db_context::DbContext,
    model::user::User,
    sync_peer::{PeerBinding, validate_identity, validate_local},
    user_db::SyncIdentity,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveToken {
    pub entity: Entity,
    pub incarnation: String,
    pub revision: i64,
    pub frontier: Vec<Dot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveChoice {
    pub destination: Option<Entity>,
    pub events: Vec<Dot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveState {
    Unavailable,
    Ready {
        destination: Option<Entity>,
        token: MoveToken,
    },
    Conflict {
        choices: Vec<MoveChoice>,
        token: MoveToken,
    },
    Paused {
        destination: Option<Entity>,
        choices: Vec<MoveChoice>,
        missing: Vec<Dependency>,
        token: MoveToken,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveResolveResult {
    Resolved(MoveState),
    Stale(MoveState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveImportResult {
    Applied(MoveState),
    /// This requests verified scope dependencies, including target genesis
    /// materialization. It does not acknowledge or store the offered event.
    NeedsDependencies {
        missing: Vec<Dependency>,
        state: MoveState,
    },
}

/// Checked database provenance only, never an authenticated wire credential.
#[derive(Debug)]
pub struct MoveBundle {
    envelope: CausalEnvelope,
    source_identity: SyncIdentity,
    root: Dot,
    wire: String,
}

impl MoveBundle {
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
    DbError::Protocol("Invalid or unsupported portable model destination history".into())
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
    user: &User,
    scope: &mut storage::Scope,
) -> Result<graph::Graph, DbError> {
    let graph = graph::load(db, scope).await?;
    if !scope.paused {
        let mut unsupported = !graph.missing.is_empty();
        for choice in &graph.choices {
            if let Some(entity) = &choice.destination {
                unsupported |= targets::by_entity(db, user, entity).await?.is_none();
            }
        }
        if unsupported {
            storage::pause(db, scope).await?;
        }
    }

    Ok(graph)
}

pub async fn get_state(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    model_id: i64,
) -> Result<MoveState, DbError> {
    validate_identity(identity)?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut tx, user, identity).await?;
    if let Some(state) = crate::causal_group_lifecycle::move_state(&mut tx, user, model_id).await? {
        tx.commit().await?;
        return Ok(state);
    }
    let state = if let Some(mut scope) = storage::load(&mut tx, user, model_id).await? {
        inspect(&mut tx, user, &mut scope).await?.state(&scope)
    } else {
        MoveState::Unavailable
    };
    tx.commit().await?;

    Ok(state)
}

pub async fn export_event(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    event: &Dot,
) -> Result<MoveBundle, DbError> {
    validate_identity(identity)?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut tx, user, identity).await?;
    let (birth, wire): (String, String) = sqlx::query_as("SELECT m.birth_id, e.envelope FROM model_move_events m JOIN causal_events e ON e.user_id = m.user_id AND e.writer_id = m.writer_id AND e.counter = m.counter WHERE m.user_id = ? AND m.writer_id = ? AND m.counter = ? AND e.encoding = 'portable_v2'")
        .bind(user.id).bind(&event.writer_id).bind(counter(event)?).fetch_optional(&mut *tx).await?.ok_or_else(invalid)?;
    let id = storage::local_id(&mut tx, user, &birth).await?;
    let mut scope = storage::load(&mut tx, user, id)
        .await?
        .ok_or_else(invalid)?;
    inspect(&mut tx, user, &mut scope).await?;
    if scope.paused {
        tx.commit().await?;
        return Err(invalid());
    }
    let envelope = causal_envelope::parse_causal_envelope(&wire).map_err(|_| invalid())?;
    if envelope.event != *event {
        return Err(invalid());
    }
    tx.commit().await?;

    Ok(MoveBundle {
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
    peer: &PeerBinding,
    bundle: &MoveBundle,
) -> Result<MoveImportResult, DbError> {
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
    let previous = inspect(&mut tx, user, &mut scope).await?;
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
        let state = previous.state(&scope);
        tx.commit().await?;

        return Ok(MoveImportResult::Applied(state));
    }
    let mut missing = previous.request_parents(&bundle.envelope);
    for dependency in &bundle.envelope.dependencies {
        if let Some(target) = targets::by_entity(&mut tx, user, &dependency.entity).await? {
            if target.root != dependency.event {
                return Err(invalid());
            }
        } else {
            missing.push(dependency.clone());
        }
    }
    if !missing.is_empty() {
        let state = previous.state(&scope);
        tx.commit().await?;

        return Ok(MoveImportResult::NeedsDependencies { missing, state });
    }
    journal::remote(&mut tx, &scope, &bundle.envelope, &bundle.wire).await?;
    let graph = graph::load(&mut tx, &scope).await?;
    if graph.frontier.len() > causal_envelope::MAX_CONTEXT_ITEMS {
        return Err(invalid());
    }
    project::apply(&mut tx, user, &mut scope, &graph, &crate::time_now()).await?;
    let state = graph.state(&scope);
    tx.commit().await?;

    Ok(MoveImportResult::Applied(state))
}

pub async fn resolve(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    model_id: i64,
    token: &MoveToken,
    destination: Option<Entity>,
) -> Result<MoveResolveResult, DbError> {
    validate_identity(identity)?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut tx, user, identity).await?;
    if let Some(state) = crate::causal_group_lifecycle::resolve_move(
        &mut tx,
        user,
        model_id,
        token,
        destination.clone(),
    )
    .await?
    {
        tx.commit().await?;
        return Ok(state);
    }
    let Some(mut scope) = storage::load(&mut tx, user, model_id).await? else {
        tx.commit().await?;
        return Ok(MoveResolveResult::Stale(MoveState::Unavailable));
    };
    let state = inspect(&mut tx, user, &mut scope).await?.state(&scope);
    let MoveState::Conflict {
        token: current,
        choices,
    } = &state
    else {
        tx.commit().await?;
        return Ok(MoveResolveResult::Stale(state));
    };
    if current != token {
        tx.commit().await?;
        return Ok(MoveResolveResult::Stale(state));
    }
    if !choices
        .iter()
        .any(|choice| choice.destination == destination)
    {
        return Err(DbError::Protocol(
            "Selected destination is not an offered choice".into(),
        ));
    }
    let target = if let Some(entity) = &destination {
        Some(
            targets::by_entity(&mut tx, user, entity)
                .await?
                .ok_or_else(invalid)?,
        )
    } else {
        None
    };
    let dependencies = dependency(target.as_ref());
    let action = Action::ModelDestinationResolved {
        to: destination,
        target_lifecycle: target.map(|target| target.root),
    };
    if !journal::local(
        &mut tx,
        &scope,
        current.frontier.clone(),
        dependencies,
        action,
    )
    .await?
    {
        storage::pause(&mut tx, &mut scope).await?;
        let state = graph::load(&mut tx, &scope).await?.state(&scope);
        tx.commit().await?;
        return Ok(MoveResolveResult::Stale(state));
    }
    let graph = graph::load(&mut tx, &scope).await?;
    project::apply(&mut tx, user, &mut scope, &graph, &crate::time_now()).await?;
    let state = graph.state(&scope);
    tx.commit().await?;

    Ok(MoveResolveResult::Resolved(state))
}

fn dependency(target: Option<&targets::Target>) -> Vec<Dependency> {
    target
        .map(|target| Dependency {
            entity: target.entity.clone(),
            event: target.root.clone(),
        })
        .into_iter()
        .collect()
}

/// Group deletion detaches members before changing the group's lifecycle row.
/// Capture affected cached assignments now, including values no longer at the
/// causal frontier, so that the detach cannot hide their unresolved choices.
pub(crate) async fn pause_group_members(
    db: &mut sqlx::SqliteConnection,
    user: &User,
    group_id: i64,
) -> Result<(), DbError> {
    sqlx::query("UPDATE model_move_scopes SET paused = 1 WHERE user_id = ? AND birth_id IN (SELECT model_birth_id FROM models WHERE model_user_id = ? AND model_group_id = ?)")
        .bind(user.id).bind(user.id).bind(group_id).execute(db).await?;

    Ok(())
}

/// Runs in the existing batch transaction before its same-destination no-op check.
/// The cached group id cannot resolve an already known destination conflict.
pub(crate) async fn try_move(
    db: &mut sqlx::SqliteConnection,
    user: &User,
    model_id: i64,
    group_id: Option<i64>,
    timestamp: &str,
) -> Result<bool, DbError> {
    let Some(mut scope) = storage::load(db, user, model_id).await? else {
        storage::invalidate(db, user, model_id).await?;
        return Ok(false);
    };
    if scope.paused {
        if scope.current_group_id != group_id {
            storage::pause(db, &mut scope).await?;
        }
        return Ok(false);
    }
    let state = inspect(db, user, &mut scope).await?.state(&scope);
    if scope.paused {
        return Ok(false);
    }
    if scope.current_group_id != group_id && !journal::capacity(db, &scope, 0, 0).await? {
        storage::pause(db, &mut scope).await?;
        return Ok(false);
    }
    let MoveState::Ready {
        destination: from,
        token,
    } = state
    else {
        return Err(DbError::Protocol(
            "Model destination choice required".into(),
        ));
    };
    if scope.current_group_id == group_id {
        return Ok(true);
    }
    let target = if let Some(id) = group_id {
        let Some(target) = targets::by_id(db, user, id).await? else {
            storage::pause(db, &mut scope).await?;
            return Ok(false);
        };
        Some(target)
    } else {
        None
    };
    let dependencies = dependency(target.as_ref());
    let action = Action::ModelMoved {
        from,
        to: target.as_ref().map(|target| target.entity.clone()),
        target_lifecycle: target.map(|target| target.root),
    };
    if !journal::local(db, &scope, token.frontier, dependencies, action).await? {
        storage::pause(db, &mut scope).await?;
        return Ok(false);
    }
    let graph = graph::load(db, &scope).await?;
    project::apply(db, user, &mut scope, &graph, timestamp).await?;

    Ok(true)
}
