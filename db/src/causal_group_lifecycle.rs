//! Combined group lifecycle capability. Network synchronization remains paused.

use sqlx::SqliteConnection;

use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, Dependency, Dot, Entity, EntityKind},
    causal_genesis::GenesisBundle,
    causal_group_rename::{NameState, ResolveResult},
    causal_model_move::{MoveResolveResult, MoveState},
    db_context::DbContext,
    model::user::User,
    user_db::SyncIdentity,
};

mod author_context;
mod graph;
mod project;
mod retention;
mod storage;
pub(crate) mod transport;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupLifecycle {
    Active,
    Deleted { deletions: Vec<Dot> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupState {
    pub entity: Entity,
    pub lifecycle: GroupLifecycle,
    pub name: NameState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationImportResult {
    Applied(GroupState),
    /// No operation is admitted or acknowledged until its external dependencies
    /// have been verified. The sender retains the original operation.
    NeedsDependencies {
        missing: Vec<Dependency>,
        state: GroupState,
    },
}

#[derive(Debug)]
pub struct OperationBundle {
    envelopes: Vec<CausalEnvelope>,
    source_identity: SyncIdentity,
    operation_id: String,
    focus_birth_id: String,
    author_context: Option<String>,
}

impl OperationBundle {
    #[must_use]
    pub fn envelopes(&self) -> &[CausalEnvelope] {
        &self.envelopes
    }

    #[must_use]
    pub const fn source_identity(&self) -> &SyncIdentity {
        &self.source_identity
    }
}

fn invalid() -> DbError {
    DbError::Protocol("Invalid or unsupported combined group lifecycle history".into())
}
fn counter(dot: &Dot) -> Result<i64, DbError> {
    dot.counter.parse().map_err(|_| invalid())
}

pub async fn enroll_pristine_group(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    id: i64,
) -> Result<GenesisBundle, DbError> {
    crate::causal_genesis::enroll(
        db,
        user,
        identity,
        EntityKind::Group,
        id,
        "group_lifecycle_v1",
    )
    .await
}
pub async fn enroll_pristine_model(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    id: i64,
) -> Result<GenesisBundle, DbError> {
    crate::causal_genesis::enroll(
        db,
        user,
        identity,
        EntityKind::Model,
        id,
        "group_lifecycle_v1",
    )
    .await
}
async fn checked(
    db: &mut SqliteConnection,
    user: &User,
) -> Result<(Vec<storage::Scope>, graph::Graph), DbError> {
    let scopes = storage::load(db, user).await?;
    let graph = graph::Graph::new(&scopes, storage::events(db, user).await?)?;

    Ok((scopes, graph))
}

pub(crate) async fn project_identity_observation(
    db: &mut SqliteConnection,
    user: &User,
) -> Result<(), DbError> {
    let (mut scopes, graph) = checked(db, user).await?;
    if scopes.iter().any(|scope| scope.paused) {
        return Err(invalid());
    }

    project::apply(db, user, &mut scopes, &graph).await
}
/// Created only after checking every existing scope inside the caller's write
/// transaction. Adding novel observed children may advance this group's counters.
pub(crate) struct IdentityExtensionGuard(storage::Scope);
impl IdentityExtensionGuard {
    pub(crate) fn materialized_group_id(&self) -> Option<i64> {
        (!self.0.purged).then_some(self.0.local_id)
    }
}

pub(crate) async fn prepare_identity_extension(
    db: &mut SqliteConnection,
    user: &User,
    entity: &Entity,
    baseline: &Dot,
) -> Result<IdentityExtensionGuard, DbError> {
    let (scopes, _graph) = checked(db, user).await?;
    let scope = scopes
        .iter()
        .find(|scope| scope.entity == *entity)
        .ok_or_else(invalid)?;
    if scopes.iter().any(|scope| scope.paused)
        || scope.project_id.is_some()
        || scope.root.event != *baseline
    {
        return Err(invalid());
    }

    Ok(IdentityExtensionGuard(scope.clone()))
}

pub(crate) async fn finish_identity_extension(
    db: &mut SqliteConnection,
    user: &User,
    guard: IdentityExtensionGuard,
) -> Result<i64, DbError> {
    let scope = guard.0;
    let current = storage::current_guard(db, user, &scope)
        .await?
        .ok_or_else(invalid)?;
    // Only this prevalidated group's counters changed during member installation;
    // all original roots, model guards and later operations remain authoritative.
    let result = sqlx::query("UPDATE group_lifecycle_scopes SET guard=? WHERE user_id=? AND entity_kind='group' AND birth_id=? AND revision=? AND paused=0")
        .bind(current).bind(user.id).bind(&scope.entity.birth_id).bind(scope.revision).execute(&mut *db).await?;
    if result.rows_affected() != 1 {
        return Err(invalid());
    }
    project_identity_observation(db, user).await?;
    sqlx::query_scalar("SELECT local_group_id FROM group_lifecycle_group_rows WHERE user_id=? AND birth_id=? AND state='present'")
        .bind(user.id).bind(&scope.entity.birth_id).fetch_one(db).await
}

/// A shared baseline may retain genuine local branches, but cannot reactivate
/// unsupported local history or substitute a row with merely matching ids.
pub(crate) async fn validate_installed_baseline(
    db: &mut SqliteConnection,
    user: &User,
    roots: &[&CausalEnvelope],
) -> Result<(), DbError> {
    let (scopes, _graph) = checked(db, user).await?;
    if roots.iter().any(|root| {
        !scopes.iter().any(|scope| {
            (scope.root == **root
                || scope
                    .independent_observations
                    .iter()
                    .any(|value| value.root == **root))
                && !scope.paused
                && !scope.purged
        })
    }) {
        return Err(invalid());
    }

    Ok(())
}

fn group(scopes: &[storage::Scope], id: i64) -> Result<&storage::Scope, DbError> {
    scopes
        .iter()
        .find(|scope| scope.entity.kind == EntityKind::Group && scope.local_id == id)
        .ok_or_else(invalid)
}
pub async fn get_state(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    id: i64,
) -> Result<GroupState, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    crate::sync_peer::validate_local(&mut tx, user, identity).await?;
    let (scopes, graph) = checked(&mut tx, user).await?;
    let state = graph.group_state(group(&scopes, id)?);
    tx.commit().await?;

    Ok(state)
}
pub async fn export_operation(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    event: &Dot,
) -> Result<OperationBundle, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    crate::sync_peer::validate_local(&mut tx, user, identity).await?;
    if !publication_ready(&mut tx, user).await? {
        tx.commit().await?;

        return Err(invalid());
    }
    let result = export_operation_in(&mut tx, user, identity, event).await?;
    tx.commit().await?;
    Ok(result)
}

pub(crate) async fn publication_ready(
    db: &mut SqliteConnection,
    user: &User,
) -> Result<bool, DbError> {
    let (scopes, _) = checked(db, user).await?;

    Ok(!scopes.iter().any(|scope| scope.paused))
}

/// Call only after checking the lifecycle scopes in this same transaction.
/// This reads the immutable original operation; it never synthesizes a snapshot.
pub(crate) async fn export_operation_in(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    event: &Dot,
) -> Result<OperationBundle, DbError> {
    let (operation_id,focus_birth_id,count,author_context):(String,String,i64,Option<String>)=sqlx::query_as("SELECT o.operation_id,o.focus_birth_id,o.event_count,o.author_context FROM group_lifecycle_events e JOIN group_lifecycle_operations o USING(user_id,operation_id) WHERE e.user_id=? AND e.writer_id=? AND e.counter=?")
        .bind(user.id).bind(&event.writer_id).bind(counter(event)?).fetch_optional(&mut *db).await?.ok_or_else(invalid)?;
    let wires:Vec<String>=sqlx::query_scalar("SELECT c.envelope FROM group_lifecycle_events e JOIN causal_events c USING(user_id,writer_id,counter) WHERE e.user_id=? AND e.operation_id=? ORDER BY e.position")
        .bind(user.id).bind(&operation_id).fetch_all(&mut *db).await?;
    if i64::try_from(wires.len()).map_err(|_| invalid())? != count {
        return Err(invalid());
    }
    let envelopes = wires
        .iter()
        .map(|wire| causal_envelope::parse_causal_envelope(wire).map_err(|_| invalid()))
        .collect::<Result<_, _>>()?;

    Ok(OperationBundle {
        envelopes,
        source_identity: identity.clone(),
        operation_id,
        focus_birth_id,
        author_context,
    })
}
pub async fn import_operation(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    peer: &crate::sync_peer::PeerBinding,
    bundle: &OperationBundle,
) -> Result<OperationImportResult, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    crate::sync_peer::validate_local(&mut tx, user, identity).await?;
    if peer.local != *identity || peer.remote != bundle.source_identity {
        return Err(invalid());
    }
    let pinned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_peer_bindings WHERE user_id=? AND local_instance_id=? AND local_account_id=? AND remote_instance_id=? AND remote_account_id=?)")
        .bind(user.id).bind(&identity.instance_id).bind(&identity.account_id).bind(&peer.remote.instance_id).bind(&peer.remote.account_id).fetch_one(&mut *tx).await?;
    if !pinned {
        return Err(invalid());
    }
    let (mut scopes, prior) = checked(&mut tx, user).await?;
    if scopes.iter().any(|scope| scope.paused) {
        tx.commit().await?;

        return Err(invalid());
    }
    let focus = scopes
        .iter()
        .position(|scope| {
            scope.entity.kind == EntityKind::Group
                && scope.entity.birth_id.as_deref() == Some(&bundle.focus_birth_id)
        })
        .ok_or_else(invalid)?;
    let context = bundle
        .author_context
        .as_deref()
        .map(|wire| author_context::parse(wire, &bundle.envelopes))
        .transpose()?;
    let mut missing = prior.missing(&bundle.envelopes)?;
    if let Some(context) = &context {
        for dependency in prior.missing_context(&context.frontier)? {
            if !missing.contains(&dependency) {
                missing.push(dependency);
            }
        }
    }
    if !missing.is_empty() {
        let state = prior.group_state(&scopes[focus]);
        tx.commit().await?;

        return Ok(OperationImportResult::NeedsDependencies { missing, state });
    }
    let existing:Option<(String,i64,Option<String>)>=sqlx::query_as("SELECT focus_birth_id,event_count,author_context FROM group_lifecycle_operations WHERE user_id=? AND operation_id=?")
        .bind(user.id).bind(&bundle.operation_id).fetch_optional(&mut *tx).await?;
    if let Some((birth, count, context)) = existing {
        let wires:Vec<String>=sqlx::query_scalar("SELECT c.envelope FROM group_lifecycle_events e JOIN causal_events c USING(user_id,writer_id,counter) WHERE e.user_id=? AND e.operation_id=? ORDER BY e.position")
            .bind(user.id).bind(&bundle.operation_id).fetch_all(&mut *tx).await?;
        let offered = bundle
            .envelopes
            .iter()
            .map(|event| causal_envelope::canonical_causal_envelope(event).map_err(|_| invalid()))
            .collect::<Result<Vec<_>, _>>()?;
        if birth != bundle.focus_birth_id
            || usize::try_from(count).map_err(|_| invalid())? != offered.len()
            || wires != offered
            || context != bundle.author_context
        {
            return Err(invalid());
        }
        let state = prior.group_state(&scopes[focus]);
        tx.commit().await?;

        return Ok(OperationImportResult::Applied(state));
    }
    if let Some(context) = &context {
        author_context::validate(
            &mut tx,
            user,
            &scopes,
            &prior,
            &bundle.focus_birth_id,
            &bundle.envelopes,
            context,
        )
        .await?;
    }
    let mut events = storage::events(&mut tx, user).await?;
    events.extend(bundle.envelopes.clone());
    let graph = graph::Graph::new(&scopes, events)?;
    storage::write(
        &mut tx,
        user,
        &bundle.operation_id,
        &bundle.focus_birth_id,
        &bundle.envelopes,
        bundle.author_context.as_deref(),
        true,
    )
    .await?;
    project::apply(&mut tx, user, &mut scopes, &graph).await?;
    let state = graph.group_state(&scopes[focus]);
    tx.commit().await?;

    Ok(OperationImportResult::Applied(state))
}

async fn event(
    db: &mut SqliteConnection,
    user: &User,
    entity: &Entity,
    parents: Vec<Dot>,
    dependencies: Vec<Dependency>,
    action: Action,
) -> Result<CausalEnvelope, DbError> {
    let (writer_id, counter) = crate::causal_clock::next(db, user.id).await?;

    Ok(CausalEnvelope {
        version: 2,
        entity: entity.clone(),
        event: Dot {
            writer_id,
            counter: counter.to_string(),
        },
        parents,
        dependencies,
        action,
    })
}
async fn commit_local(
    db: &mut SqliteConnection,
    user: &User,
    scopes: &mut [storage::Scope],
    focus: &str,
    events: Vec<CausalEnvelope>,
) -> Result<bool, DbError> {
    let mut all = storage::events(db, user).await?;
    let prior = graph::Graph::new(scopes, all.clone())?;
    let author_context = author_context::capture(scopes, &prior, &events)?;
    all.extend(events.clone());
    if !fits(&all)? || !author_context::fits(db, user, &all, author_context.as_deref()).await? {
        let first = events.first().ok_or_else(invalid)?;
        let (writer, current) = crate::causal_clock::read(db, user.id).await?;
        if writer != first.event.writer_id {
            return Err(invalid());
        }
        crate::causal_clock::set(
            db,
            user.id,
            &writer,
            current,
            counter(&first.event)?.checked_sub(1).ok_or_else(invalid)?,
        )
        .await?;
        storage::pause(db, user).await?;
        for scope in scopes {
            scope.paused = true;
        }

        return Ok(false);
    }
    let graph = graph::Graph::new(scopes, all)?;
    storage::write(
        db,
        user,
        &crate::random_hex_32(),
        focus,
        &events,
        author_context.as_deref(),
        false,
    )
    .await?;
    project::apply(db, user, scopes, &graph).await?;

    Ok(true)
}
fn fits(events: &[CausalEnvelope]) -> Result<bool, DbError> {
    if events.len() > crate::causal_store::MAX_OWNER_EVENTS {
        return Ok(false);
    }
    let mut bytes = 0_usize;
    let mut edges = 0_usize;
    for event in events {
        if event.parents.len() > causal_envelope::MAX_CONTEXT_ITEMS
            || event.dependencies.len() > causal_envelope::MAX_CONTEXT_ITEMS
            || matches!(&event.action,Action::GroupRenamed{name}|Action::GroupNameResolved{name} if name.len()>causal_envelope::MAX_NAME_BYTES)
        {
            return Ok(false);
        }
        bytes = bytes.saturating_add(
            causal_envelope::canonical_causal_envelope(event)
                .map_err(|_| invalid())?
                .len(),
        );
        edges = edges
            .saturating_add(event.parents.len())
            .saturating_add(event.dependencies.len());
    }

    Ok(bytes <= crate::causal_store::MAX_OWNER_BYTES
        && edges <= crate::causal_store::MAX_OWNER_DEPENDENCIES)
}

pub(crate) async fn try_edit(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
    name: &str,
    timestamp: &str,
    rekey: bool,
) -> Result<bool, DbError> {
    if !storage::enrolled(db, user, "group", id).await? {
        return Ok(false);
    }
    if rekey {
        storage::pause(db, user).await?;

        return Ok(false);
    }
    let (mut scopes, graph) = checked(db, user).await?;
    if scopes.iter().any(|scope| scope.paused) {
        return Ok(false);
    }

    let scope = group(&scopes, id)?;
    let names = graph.names(&scope.entity, None);
    if names.len() != 1 {
        return Err(DbError::Protocol("Group name choice required".into()));
    }
    if names[0].name == name {
        sqlx::query(
            "UPDATE models_group SET group_last_modified=? WHERE group_id=? AND group_user_id=?",
        )
        .bind(timestamp)
        .bind(id)
        .bind(user.id)
        .execute(db)
        .await?;

        return Ok(true);
    }
    let focus = scope.entity.birth_id.clone().ok_or_else(invalid)?;
    let rename = event(
        db,
        user,
        &scope.entity,
        graph.frontier(&scope.entity, None),
        Vec::new(),
        Action::GroupRenamed { name: name.into() },
    )
    .await?;
    let committed = commit_local(db, user, &mut scopes, &focus, vec![rename]).await?;
    if committed {
        sqlx::query(
            "UPDATE models_group SET group_last_modified=? WHERE group_id=? AND group_user_id=?",
        )
        .bind(timestamp)
        .bind(id)
        .bind(user.id)
        .execute(db)
        .await?;
    }

    Ok(committed)
}

pub(crate) async fn try_move(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
    destination: Option<i64>,
) -> Result<bool, DbError> {
    if !storage::enrolled(db, user, "model", id).await? {
        return Ok(false);
    }
    let (mut scopes, graph) = checked(db, user).await?;
    if scopes.iter().any(|scope| scope.paused) {
        return Ok(false);
    }

    let scope = scopes
        .iter()
        .find(|scope| scope.entity.kind == EntityKind::Model && scope.local_id == id)
        .ok_or_else(invalid)?;
    let choices = graph.moves(&scope.entity, None);
    if choices.len() != 1 {
        return Err(DbError::Protocol(
            "Model destination choice required".into(),
        ));
    }
    let target = if let Some(id) = destination {
        let Ok(target) = group(&scopes, id) else {
            storage::pause(db, user).await?;

            return Ok(false);
        };

        Some(target)
    } else {
        None
    };
    let to = target.map(|scope| scope.entity.clone());
    if choices[0].destination == to {
        return Ok(true);
    }
    let focus = to
        .as_ref()
        .or(choices[0].destination.as_ref())
        .and_then(|entity| entity.birth_id.clone())
        .ok_or_else(invalid)?;
    // Capture observed group context as well as model context. A later concurrent
    // keep must not rewrite what 'from' meant when this operation was authored.
    let dependencies = group_context(&scopes, &graph)?;
    let lifecycle = target
        .map(|scope| graph.lifecycle(&scope.entity))
        .transpose()?;
    let movement = event(
        db,
        user,
        &scope.entity,
        graph.frontier(&scope.entity, None),
        dependencies,
        Action::ModelMoved {
            from: choices[0].destination.clone(),
            to: to.clone(),
            target_lifecycle: lifecycle,
        },
    )
    .await?;
    let mut events = vec![movement.clone()];
    if let Some(target) = target {
        events.push(
            event(
                db,
                user,
                &target.entity,
                graph.frontier(&target.entity, None),
                vec![Dependency {
                    entity: scope.entity.clone(),
                    event: movement.event.clone(),
                }],
                Action::GroupMemberAdded {
                    model: scope.entity.clone(),
                    membership_event: movement.event,
                },
            )
            .await?,
        );
    }

    commit_local(db, user, &mut scopes, &focus, events).await
}

pub(crate) async fn try_delete(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
) -> Result<bool, DbError> {
    if !storage::enrolled(db, user, "group", id).await? {
        return Ok(false);
    }
    let (mut scopes, graph) = checked(db, user).await?;
    if scopes.iter().any(|scope| scope.paused) {
        return Ok(false);
    }

    let scope = group(&scopes, id)?;
    let focus = scope.entity.birth_id.clone().ok_or_else(invalid)?;
    let deletion = event(
        db,
        user,
        &scope.entity,
        graph.frontier(&scope.entity, None),
        Vec::new(),
        Action::GroupDeleted {
            deletion_operation_id: crate::random_hex_32(),
        },
    )
    .await?;
    let mut events = vec![deletion.clone()];
    for model in scopes
        .iter()
        .filter(|scope| scope.entity.kind == EntityKind::Model)
    {
        if graph
            .moves(&model.entity, None)
            .iter()
            .any(|choice| choice.destination.as_ref() == Some(&scope.entity))
        {
            events.push(
                event(
                    db,
                    user,
                    &model.entity,
                    graph.frontier(&model.entity, None),
                    vec![Dependency {
                        entity: scope.entity.clone(),
                        event: deletion.event.clone(),
                    }],
                    Action::ModelDetached {
                        group: scope.entity.clone(),
                        deletion: deletion.event.clone(),
                    },
                )
                .await?,
            );
            sqlx::query(
                "INSERT OR IGNORE INTO group_recovery_members(group_id,model_id) VALUES(?,?)",
            )
            .bind(id)
            .bind(model.local_id)
            .execute(&mut *db)
            .await?;
        }
    }

    commit_local(db, user, &mut scopes, &focus, events).await
}

pub(crate) async fn name_state(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
) -> Result<Option<NameState>, DbError> {
    if !storage::enrolled(db, user, "group", id).await? {
        return Ok(None);
    }
    let (scopes, graph) = checked(db, user).await?;

    Ok(Some(graph.group_state(group(&scopes, id)?).name))
}
pub(crate) async fn move_state(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
) -> Result<Option<MoveState>, DbError> {
    if !storage::enrolled(db, user, "model", id).await? {
        return Ok(None);
    }
    let (scopes, graph) = checked(db, user).await?;
    let scope = scopes
        .iter()
        .find(|scope| scope.entity.kind == EntityKind::Model && scope.local_id == id)
        .ok_or_else(invalid)?;

    Ok(Some(graph.move_state(scope)))
}
pub(crate) async fn resolve_name(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
    token: &crate::causal_group_rename::NameToken,
    name: &str,
) -> Result<Option<ResolveResult>, DbError> {
    if !storage::enrolled(db, user, "group", id).await? {
        return Ok(None);
    }
    let (mut scopes, graph) = checked(db, user).await?;
    let scope = group(&scopes, id)?;
    let state = graph.group_state(scope).name;
    let NameState::Conflict {
        token: current,
        choices,
    } = &state
    else {
        return Ok(Some(ResolveResult::Stale(state)));
    };
    if current != token {
        return Ok(Some(ResolveResult::Stale(state)));
    }
    if !choices.iter().any(|choice| choice.name == name) {
        return Err(DbError::Protocol(
            "Selected name is not an offered choice".into(),
        ));
    }
    let focus = scope.entity.birth_id.clone().ok_or_else(invalid)?;
    let mut parents = current.frontier.clone();
    for dot in choices.iter().flat_map(|choice| choice.events.iter()) {
        if !parents.contains(dot) {
            parents.push(dot.clone());
        }
    }
    let resolution = event(
        db,
        user,
        &scope.entity,
        parents,
        Vec::new(),
        Action::GroupNameResolved { name: name.into() },
    )
    .await?;
    if !commit_local(db, user, &mut scopes, &focus, vec![resolution]).await? {
        return Ok(Some(ResolveResult::Stale(
            graph.group_state(group(&scopes, id)?).name,
        )));
    }
    let graph = graph::Graph::new(&scopes, storage::events(db, user).await?)?;

    Ok(Some(ResolveResult::Resolved(
        graph.group_state(group(&scopes, id)?).name,
    )))
}
pub(crate) async fn resolve_move(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
    token: &crate::causal_model_move::MoveToken,
    destination: Option<Entity>,
) -> Result<Option<MoveResolveResult>, DbError> {
    if !storage::enrolled(db, user, "model", id).await? {
        return Ok(None);
    }
    let (mut scopes, graph) = checked(db, user).await?;
    let index = scopes
        .iter()
        .position(|scope| scope.entity.kind == EntityKind::Model && scope.local_id == id)
        .ok_or_else(invalid)?;
    let scope = &scopes[index];
    let state = graph.move_state(scope);
    let MoveState::Conflict {
        token: current,
        choices,
    } = &state
    else {
        return Ok(Some(MoveResolveResult::Stale(state)));
    };
    if current != token {
        return Ok(Some(MoveResolveResult::Stale(state)));
    }
    if !choices
        .iter()
        .any(|choice| choice.destination == destination)
    {
        return Err(DbError::Protocol(
            "Selected destination is not an offered choice".into(),
        ));
    }
    let target = destination
        .as_ref()
        .map(|entity| {
            scopes
                .iter()
                .find(|scope| scope.entity == *entity)
                .ok_or_else(invalid)
        })
        .transpose()?;
    let focus = destination
        .as_ref()
        .or_else(|| {
            choices
                .iter()
                .find_map(|choice| choice.destination.as_ref())
        })
        .and_then(|entity| entity.birth_id.clone())
        .ok_or_else(invalid)?;
    let mut parents = current.frontier.clone();
    for dot in choices.iter().flat_map(|choice| choice.events.iter()) {
        if !parents.contains(dot) {
            parents.push(dot.clone());
        }
    }
    let dependencies = group_context(&scopes, &graph)?;
    let resolution = event(
        db,
        user,
        &scope.entity,
        parents,
        dependencies,
        Action::ModelDestinationResolved {
            to: destination.clone(),
            target_lifecycle: target
                .map(|scope| graph.lifecycle(&scope.entity))
                .transpose()?,
        },
    )
    .await?;
    let mut events = vec![resolution.clone()];
    if let Some(target) = target {
        events.push(
            event(
                db,
                user,
                &target.entity,
                graph.frontier(&target.entity, None),
                vec![Dependency {
                    entity: scope.entity.clone(),
                    event: resolution.event.clone(),
                }],
                Action::GroupMemberAdded {
                    model: scope.entity.clone(),
                    membership_event: resolution.event,
                },
            )
            .await?,
        );
    }
    if !commit_local(db, user, &mut scopes, &focus, events).await? {
        return Ok(Some(MoveResolveResult::Stale(
            graph.move_state(&scopes[index]),
        )));
    }
    let graph = graph::Graph::new(&scopes, storage::events(db, user).await?)?;

    Ok(Some(MoveResolveResult::Resolved(
        graph.move_state(&scopes[index]),
    )))
}

fn group_context(
    scopes: &[storage::Scope],
    graph: &graph::Graph,
) -> Result<Vec<Dependency>, DbError> {
    let mut dependencies = Vec::new();
    for scope in scopes
        .iter()
        .filter(|scope| scope.entity.kind == EntityKind::Group)
    {
        let mut frontier = graph.frontier(&scope.entity, None);
        let lifecycle = graph.lifecycle(&scope.entity)?;
        if !frontier.contains(&lifecycle) {
            frontier.push(lifecycle);
        }
        dependencies.extend(frontier.into_iter().map(|event| Dependency {
            entity: scope.entity.clone(),
            event,
        }));
    }

    Ok(dependencies)
}

pub(crate) async fn try_restore(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
    operation: &str,
) -> Result<bool, DbError> {
    if !storage::enrolled(db, user, "group", id).await? {
        return Ok(false);
    }
    let (mut scopes, graph) = checked(db, user).await?;
    if scopes.iter().any(|scope| scope.paused) {
        // Legacy restoration may keep its established local behavior, but it
        // cannot use a cached NULL destination to resolve a retained conflict.
        for model in scopes
            .iter()
            .filter(|scope| scope.entity.kind == EntityKind::Model)
        {
            if graph.moves(&model.entity, None).len() != 1 {
                sqlx::query("DELETE FROM group_recovery_members WHERE group_id=? AND model_id=?")
                    .bind(id)
                    .bind(model.local_id)
                    .execute(&mut *db)
                    .await?;
            }
        }

        return Ok(false);
    }
    let scope = group(&scopes, id)?;
    let focus = scope.entity.birth_id.clone().ok_or_else(invalid)?;
    let deletion=graph.events.iter().find(|event|event.entity==scope.entity&&matches!(&event.action,Action::GroupDeleted{deletion_operation_id} if deletion_operation_id==operation)).ok_or_else(invalid)?;
    let restoration = event(
        db,
        user,
        &scope.entity,
        graph.frontier(&scope.entity, None),
        vec![Dependency {
            entity: scope.entity.clone(),
            event: deletion.event.clone(),
        }],
        Action::GroupRestored {
            deletion: deletion.event.clone(),
        },
    )
    .await?;
    let mut events = vec![restoration.clone()];
    let mut group_parent = restoration.event.clone();
    for model in scopes
        .iter()
        .filter(|scope| scope.entity.kind == EntityKind::Model)
    {
        let former=graph.events.iter().any(|event|event.entity==model.entity&&matches!(&event.action,Action::ModelDetached{group,deletion:dot} if group==&scope.entity&&dot==&deletion.event));
        let choices = graph.moves(&model.entity, None);
        if !former || choices.len() != 1 || choices[0].destination.is_some() {
            continue;
        }
        let restored = event(
            db,
            user,
            &model.entity,
            graph.frontier(&model.entity, None),
            vec![
                Dependency {
                    entity: scope.entity.clone(),
                    event: deletion.event.clone(),
                },
                Dependency {
                    entity: scope.entity.clone(),
                    event: restoration.event.clone(),
                },
            ],
            Action::ModelRestored {
                group: scope.entity.clone(),
                deletion: deletion.event.clone(),
                restoration: restoration.event.clone(),
            },
        )
        .await?;
        let added = event(
            db,
            user,
            &scope.entity,
            vec![group_parent],
            vec![Dependency {
                entity: model.entity.clone(),
                event: restored.event.clone(),
            }],
            Action::GroupMemberAdded {
                model: model.entity.clone(),
                membership_event: restored.event.clone(),
            },
        )
        .await?;
        group_parent = added.event.clone();
        events.extend([restored, added]);
    }

    commit_local(db, user, &mut scopes, &focus, events).await
}

pub(crate) async fn purge_expired(db: &DbContext, retention_days: u32) -> Result<u64, DbError> {
    retention::purge(db, retention_days).await
}

pub(crate) async fn restore_retry(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
    operation: &str,
) -> Result<bool, DbError> {
    if !storage::enrolled(db, user, "group", id).await? {
        return Ok(false);
    }
    let (scopes, graph) = checked(db, user).await?;
    let scope = group(&scopes, id)?;
    if scope.purged
        || !graph.active(&scope.entity, None)
        || !graph.restored_operation(&scope.entity, operation)
    {
        return Ok(false);
    }
    // An unrelated unsupported model change may pause the owner graph after a
    // successful restore. Acknowledging its exact retry does not apply history,
    // but the group's own guard must still rule out an unrecorded lifecycle ABA.
    let expected:String=sqlx::query_scalar("SELECT guard FROM group_lifecycle_scopes WHERE user_id=? AND entity_kind='group' AND birth_id=?")
        .bind(user.id).bind(&scope.entity.birth_id).fetch_one(&mut *db).await?;
    if storage::current_guard(db, user, scope).await?.as_deref() != Some(expected.as_str()) {
        return Ok(false);
    }
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_deletion_records WHERE user_id=? AND group_global_id=? AND group_birth_id=? AND operation_id=? AND restored_at IS NOT NULL)")
        .bind(user.id).bind(&scope.entity.global_id).bind(&scope.entity.birth_id).bind(operation).fetch_one(db).await
}
