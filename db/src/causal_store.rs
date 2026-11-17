use std::collections::{HashMap, HashSet};

use sqlx::SqliteConnection;

use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, Dependency, Dot, Entity},
    db_context::DbContext,
    group_history::ReplicaPrincipal,
    model::user::User,
    sync_peer::{validate_identity, validate_local},
    user_db::SyncIdentity,
};

pub const MAX_OWNER_EVENTS: usize = 4096;
pub const MAX_OWNER_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_OWNER_DEPENDENCIES: usize = 32768;

#[derive(Debug, PartialEq, Eq)]
pub enum ReceiveResult {
    AcceptedReady,
    AcceptedPending {
        missing: Vec<Dependency>,
    },
    AcceptedIdentityConflict {
        missing: Vec<Dependency>,
    },
    AcceptedIdentityUnknown {
        missing: Vec<Dependency>,
    },
    /// Exact receipt acknowledgement only; read again for current dependency/identity status.
    AlreadyPresent,
}

#[derive(Debug, PartialEq, Eq)]
pub enum IdentityStatus {
    /// No known conflicting claim; this does not authenticate or adopt the identity.
    Uncontested,
    Conflict,
    Unknown,
}

pub struct StoredEnvelope {
    pub envelope: CausalEnvelope,
    pub missing: Vec<Dependency>,
    pub identity_status: IdentityStatus,
}

type EventKey = (String, i64);

/// Records receipts only. Writer registration is bookkeeping, not authentication;
/// ready means dependencies are available, not that ancestry is complete or applied.
pub async fn receive(
    db: &DbContext,
    user: &User,
    expected_local: &SyncIdentity,
    principal: &ReplicaPrincipal,
    wire: &str,
) -> Result<ReceiveResult, DbError> {
    validate_identity(expected_local)?;
    let envelope = parse(wire)?;
    let canonical = causal_envelope::canonical_causal_envelope(&envelope).map_err(|_| invalid())?;
    let (writer, counter) = event_key(&envelope.event)?;
    if principal.owner_user_id != user.id || principal.replica_id != writer {
        return Err(invalid());
    }
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut transaction, user, expected_local).await?;
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM causal_replica_owners WHERE user_id = ? AND replica_id = ?) AND NOT EXISTS(SELECT 1 FROM local_causal_writers WHERE replica_id = ?) AND NOT EXISTS(SELECT 1 FROM group_causal_replica WHERE replica_id = ?)")
        .bind(user.id).bind(&writer).bind(&writer).bind(&writer).fetch_one(&mut *transaction).await?;
    if !allowed {
        return Err(invalid());
    }
    let prior: Option<(i64, String)> = sqlx::query_as(
        "SELECT e.user_id, e.envelope FROM causal_event_receipts r JOIN causal_events e ON e.user_id = r.user_id AND e.writer_id = r.writer_id AND e.counter = r.counter WHERE r.writer_id = ? AND r.counter = ?",
    )
    .bind(&writer)
    .bind(counter)
    .fetch_optional(&mut *transaction)
    .await?;
    if let Some((owner, value)) = prior {
        if owner != user.id || value != canonical {
            return Err(invalid());
        }
        transaction.commit().await?;

        return Ok(ReceiveResult::AlreadyPresent);
    }
    if legacy_dot_exists(&mut transaction, &envelope.event).await?
        || sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM causal_events WHERE writer_id = ? AND counter = ?)",
        )
        .bind(&writer)
        .bind(counter)
        .fetch_one(&mut *transaction)
        .await?
    {
        return Err(invalid());
    }
    let references = references(&envelope)?;
    check_quota(&mut transaction, user.id, canonical.len(), references.len()).await?;
    sqlx::query(
        "INSERT INTO causal_events (user_id, writer_id, counter, envelope) VALUES (?, ?, ?, ?)",
    )
    .bind(user.id)
    .bind(&writer)
    .bind(counter)
    .bind(canonical)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("INSERT INTO causal_event_receipts (user_id, writer_id, counter) VALUES (?, ?, ?)")
        .bind(user.id)
        .bind(&writer)
        .bind(counter)
        .execute(&mut *transaction)
        .await?;
    for reference in references {
        let (dependency_writer, dependency_counter) = event_key(&reference.event)?;
        sqlx::query("INSERT INTO causal_receipt_dependencies (user_id, writer_id, counter, dependency_writer_id, dependency_counter) VALUES (?, ?, ?, ?, ?)")
            .bind(user.id).bind(&writer).bind(counter).bind(dependency_writer).bind(dependency_counter).execute(&mut *transaction).await?;
    }
    // Retain competing authored claims before assessing them; neither arrival wins identity.
    let (missing, identity_status) =
        inspect_dependencies(&mut transaction, user.id, &envelope).await?;
    transaction.commit().await?;

    match identity_status {
        IdentityStatus::Conflict => Ok(ReceiveResult::AcceptedIdentityConflict { missing }),
        IdentityStatus::Unknown => Ok(ReceiveResult::AcceptedIdentityUnknown { missing }),
        IdentityStatus::Uncontested if missing.is_empty() => Ok(ReceiveResult::AcceptedReady),
        IdentityStatus::Uncontested => Ok(ReceiveResult::AcceptedPending { missing }),
    }
}

pub async fn get_received(
    db: &DbContext,
    user: &User,
    expected_local: &SyncIdentity,
    dot: &Dot,
) -> Result<Option<StoredEnvelope>, DbError> {
    validate_identity(expected_local)?;
    let (writer, counter) = event_key(dot)?;
    let mut transaction = db.begin().await?;
    validate_local(&mut transaction, user, expected_local).await?;
    let value: Option<String> = sqlx::query_scalar("SELECT e.envelope FROM causal_event_receipts r JOIN causal_events e ON e.user_id = r.user_id AND e.writer_id = r.writer_id AND e.counter = r.counter WHERE r.user_id = ? AND r.writer_id = ? AND r.counter = ?")
        .bind(user.id).bind(writer).bind(counter).fetch_optional(&mut *transaction).await?;
    let Some(value) = value else {
        return Ok(None);
    };
    let envelope = parse(&value)?;
    if envelope.event != *dot {
        return Err(invalid());
    }
    let (missing, identity_status) =
        inspect_dependencies(&mut transaction, user.id, &envelope).await?;
    transaction.commit().await?;

    Ok(Some(StoredEnvelope {
        envelope,
        missing,
        identity_status,
    }))
}

fn invalid() -> DbError {
    DbError::Protocol("Invalid or unsupported causal receipt".into())
}

fn parse(value: &str) -> Result<CausalEnvelope, DbError> {
    causal_envelope::parse_causal_envelope(value).map_err(|_| invalid())
}

fn event_key(dot: &Dot) -> Result<EventKey, DbError> {
    if dot.writer_id.len() != 32
        || !dot
            .writer_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || dot.counter.starts_with('0')
        || !dot.counter.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid());
    }
    let counter = dot
        .counter
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(invalid)?;

    Ok((dot.writer_id.clone(), counter))
}

fn references(envelope: &CausalEnvelope) -> Result<Vec<Dependency>, DbError> {
    let mut references = envelope.dependencies.clone();
    for parent in &envelope.parents {
        if let Some(existing) = references
            .iter()
            .find(|reference| reference.event == *parent)
        {
            if existing.entity != envelope.entity {
                return Err(invalid());
            }
        } else {
            references.push(Dependency {
                entity: envelope.entity.clone(),
                event: parent.clone(),
            });
        }
    }

    Ok(references)
}

pub(crate) async fn check_quota(
    db: &mut SqliteConnection,
    owner: i64,
    bytes: usize,
    dependencies: usize,
) -> Result<(), DbError> {
    let (events, used_bytes, edges): (i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_event_receipts WHERE user_id = ?), (SELECT COALESCE(SUM(length(CAST(e.envelope AS BLOB))), 0) FROM causal_event_receipts r JOIN causal_events e ON e.user_id = r.user_id AND e.writer_id = r.writer_id AND e.counter = r.counter WHERE r.user_id = ?), (SELECT COUNT(*) FROM causal_receipt_dependencies WHERE user_id = ?)")
        .bind(owner).bind(owner).bind(owner).fetch_one(db).await?;
    if usize::try_from(events).map_err(|_| invalid())? >= MAX_OWNER_EVENTS
        || usize::try_from(used_bytes)
            .map_err(|_| invalid())?
            .checked_add(bytes)
            .is_none_or(|total| total > MAX_OWNER_BYTES)
        || usize::try_from(edges)
            .map_err(|_| invalid())?
            .checked_add(dependencies)
            .is_none_or(|total| total > MAX_OWNER_DEPENDENCIES)
    {
        return Err(DbError::Protocol("Causal receipt quota exceeded".into()));
    }

    Ok(())
}

pub(crate) async fn identity_status(
    db: &mut SqliteConnection,
    owner: i64,
    entity: &Entity,
) -> Result<IdentityStatus, DbError> {
    let Some(birth) = &entity.birth_id else {
        return Ok(IdentityStatus::Unknown);
    };
    let kind = match entity.kind {
        causal_envelope::EntityKind::Group => "group",
        causal_envelope::EntityKind::Model => "model",
    };
    let conflict: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM entity_births WHERE user_id = ? AND entity_kind = ? AND birth_id = ? AND global_id != ?)
         OR EXISTS(SELECT 1 FROM entity_births WHERE user_id = ? AND entity_kind = ? AND global_id = ? AND birth_id != ?)
         OR EXISTS(SELECT 1 FROM causal_events JOIN causal_event_receipts USING (user_id, writer_id, counter) WHERE user_id = ? AND encoding = 'portable_v2' AND json_extract(envelope, '$.entity.kind') = ? AND json_extract(envelope, '$.entity.global_id') = ? AND json_extract(envelope, '$.entity.birth_id') IS NOT NULL AND json_extract(envelope, '$.entity.birth_id') != ?)
         OR EXISTS(SELECT 1 FROM causal_events JOIN causal_event_receipts USING (user_id, writer_id, counter) WHERE user_id = ? AND encoding = 'portable_v2' AND json_extract(envelope, '$.entity.kind') = ? AND json_extract(envelope, '$.entity.birth_id') = ? AND json_extract(envelope, '$.entity.birth_id') IS NOT NULL AND json_extract(envelope, '$.entity.global_id') != ?)",
    )
    .bind(owner).bind(kind).bind(birth).bind(&entity.global_id)
    .bind(owner).bind(kind).bind(&entity.global_id).bind(birth)
    .bind(owner).bind(kind).bind(&entity.global_id).bind(birth)
    .bind(owner).bind(kind).bind(birth).bind(&entity.global_id)
    .fetch_one(&mut *db).await?;
    if conflict {
        return Ok(IdentityStatus::Conflict);
    }
    // A matching legacy global id cannot establish shared identity without adoption evidence.
    // A local creation under a previous global id is retained evidence of an unresolved rekey,
    // not another remote claimant and not proof that the new identity has complete history.
    let unknown: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM causal_events JOIN causal_event_receipts USING (user_id, writer_id, counter) WHERE user_id = ? AND encoding = 'portable_v2' AND json_extract(envelope, '$.entity.kind') = ? AND json_extract(envelope, '$.entity.global_id') = ? AND json_extract(envelope, '$.entity.birth_id') IS NULL)
         OR EXISTS(SELECT 1 FROM models_group WHERE ? = 'group' AND group_user_id = ? AND group_unique_global_id = ? AND group_birth_id IS NULL)
         OR EXISTS(SELECT 1 FROM models WHERE ? = 'model' AND model_user_id = ? AND model_unique_global_id = ? AND model_birth_id IS NULL)
         OR EXISTS(SELECT 1 FROM group_deletion_records WHERE ? = 'group' AND user_id = ? AND group_global_id = ? AND group_birth_id IS NULL)
         OR EXISTS(SELECT 1 FROM group_sync_heads WHERE ? = 'group' AND user_id = ? AND group_global_id = ? AND identity_birth_id IS NOT ?)
         OR EXISTS(SELECT 1 FROM membership_causal_events m WHERE ? = 'model' AND user_id = ? AND model_global_id = ? AND model_birth_id IS NULL AND NOT EXISTS(SELECT 1 FROM model_identity_adoptions a JOIN entity_births b ON b.user_id=a.user_id AND b.entity_kind='model' AND b.birth_id=a.birth_id AND b.global_id=a.model_global_id WHERE a.user_id=m.user_id AND a.model_global_id=m.model_global_id AND a.model_incarnation=m.model_incarnation AND a.birth_id=?))
         OR EXISTS(SELECT 1 FROM membership_causal_events WHERE ? = 'model' AND user_id = ? AND model_global_id = ? AND model_birth_id < ?)
         OR EXISTS(SELECT 1 FROM membership_causal_events WHERE ? = 'model' AND user_id = ? AND model_global_id = ? AND model_birth_id > ?)
         OR EXISTS(SELECT 1 FROM unknown_model_identity_tombstones t WHERE ? = 'model' AND t.user_id = ? AND t.model_global_id = ? AND (NOT EXISTS(SELECT 1 FROM unknown_model_identity_evidence e WHERE e.user_id=t.user_id AND e.model_global_id=t.model_global_id) OR EXISTS(SELECT 1 FROM unknown_model_identity_evidence e WHERE e.user_id=t.user_id AND e.model_global_id=t.model_global_id AND NOT EXISTS(SELECT 1 FROM model_identity_separation_evidence c WHERE c.user_id=e.user_id AND c.evidence_id=e.evidence_id AND c.remote_global_id=t.model_global_id AND c.remote_birth_id=?))))
         OR EXISTS(SELECT 1 FROM local_creation_records c JOIN causal_events e USING (user_id, writer_id, counter) WHERE c.user_id = ? AND c.entity_kind = ? AND c.birth_id = ? AND json_extract(e.envelope, '$.entity.global_id') != ?)",
    )
    .bind(owner).bind(kind).bind(&entity.global_id)
    .bind(kind).bind(owner).bind(&entity.global_id)
    .bind(kind).bind(owner).bind(&entity.global_id)
    .bind(kind).bind(owner).bind(&entity.global_id)
    .bind(kind).bind(owner).bind(&entity.global_id).bind(birth)
    .bind(kind).bind(owner).bind(&entity.global_id).bind(birth)
    .bind(kind).bind(owner).bind(&entity.global_id).bind(birth)
    .bind(kind).bind(owner).bind(&entity.global_id).bind(birth)
    .bind(kind).bind(owner).bind(&entity.global_id).bind(birth)
    .bind(owner).bind(kind).bind(birth).bind(&entity.global_id)
    .fetch_one(db).await?;

    if unknown {
        Ok(IdentityStatus::Unknown)
    } else {
        Ok(IdentityStatus::Uncontested)
    }
}

async fn legacy_dot_exists(db: &mut SqliteConnection, dot: &Dot) -> Result<bool, DbError> {
    let (writer, counter) = event_key(dot)?;

    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_causal_events WHERE replica_id = ? AND counter = ?) OR EXISTS(SELECT 1 FROM membership_causal_events WHERE replica_id = ? AND counter = ?) OR EXISTS(SELECT 1 FROM received_group_renames WHERE replica_id = ? AND counter = ?)")
        .bind(&writer).bind(counter).bind(&writer).bind(counter).bind(&writer).bind(counter).fetch_one(db).await
}

async fn load_dependency(
    db: &mut SqliteConnection,
    owner: i64,
    dot: &Dot,
) -> Result<Option<(CausalEnvelope, usize)>, DbError> {
    let (writer, counter) = event_key(dot)?;
    if legacy_dot_exists(db, dot).await? {
        // Legacy dots lack the portable entity/type proof needed to resolve this dependency.
        return Err(invalid());
    }
    let row: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT user_id, encoding, envelope FROM causal_events WHERE writer_id = ? AND counter = ?",
    )
    .bind(&writer)
    .bind(counter)
    .fetch_optional(db)
    .await?;
    let Some((row_owner, encoding, value)) = row.as_ref() else {
        return Ok(None);
    };
    // A shared dot namespace does not make incomplete local facts portable ancestry.
    if encoding != "portable_v2" {
        return Err(invalid());
    }
    let envelope = parse(value)?;
    if *row_owner != owner || envelope.event != *dot {
        return Err(invalid());
    }

    Ok(Some((envelope, value.len())))
}

fn validate_dependency(
    child: &CausalEnvelope,
    reference: &Dependency,
    parent: &CausalEnvelope,
) -> Result<(), DbError> {
    if reference.entity != parent.entity {
        return Err(invalid());
    }
    let correct_type = match &child.action {
        Action::GroupRestored { deletion } | Action::ModelDetached { deletion, .. }
            if *deletion == reference.event =>
        {
            matches!(parent.action, Action::GroupDeleted { .. })
        }
        Action::GroupMemberAdded {
            membership_event, ..
        } if *membership_event == reference.event => match &parent.action {
            Action::ModelMoved {
                to: Some(group), ..
            }
            | Action::ModelDestinationResolved {
                to: Some(group), ..
            }
            | Action::ModelRestored { group, .. } => *group == child.entity,
            _ => false,
        },
        Action::ModelMoved {
            target_lifecycle: Some(lifecycle),
            ..
        }
        | Action::ModelDestinationResolved {
            target_lifecycle: Some(lifecycle),
            ..
        } if *lifecycle == reference.event => matches!(
            parent.action,
            Action::GroupCreated { .. }
                | Action::GroupRestored { .. }
                | Action::LegacyObserved { .. }
        ),
        Action::ModelRestored {
            deletion,
            restoration,
            ..
        } => {
            if *deletion == reference.event {
                matches!(parent.action, Action::GroupDeleted { .. })
            } else if *restoration == reference.event {
                matches!(&parent.action, Action::GroupRestored { deletion: observed } if observed == deletion)
            } else {
                true
            }
        }
        _ => true,
    };
    if !correct_type {
        return Err(invalid());
    }

    Ok(())
}

async fn inspect_dependencies(
    db: &mut SqliteConnection,
    owner: i64,
    root: &CausalEnvelope,
) -> Result<(Vec<Dependency>, IdentityStatus), DbError> {
    let root_key = event_key(&root.event)?;
    let mut envelopes = HashMap::from([(root_key.clone(), root.clone())]);
    let mut stack = vec![(root_key, false)];
    let mut active = HashSet::new();
    let mut completed = HashSet::new();
    let mut missing: HashMap<EventKey, Dependency> = HashMap::new();
    let mut edges = 0_usize;
    let mut checked_entities = HashSet::new();
    let mut graph_identity = IdentityStatus::Uncontested;
    // Local creation evidence is outside receipt quotas, so bound the combined traversal too.
    let mut loaded_bytes = causal_envelope::canonical_causal_envelope(root)
        .map_err(|_| invalid())?
        .len();
    while let Some((key, finishing)) = stack.pop() {
        if finishing {
            active.remove(&key);
            completed.insert(key);
            continue;
        }
        if completed.contains(&key) {
            continue;
        }
        if !active.insert(key.clone()) {
            return Err(invalid());
        }
        stack.push((key.clone(), true));
        let envelope = envelopes.get(&key).ok_or_else(invalid)?.clone();
        let entity_key = (
            matches!(envelope.entity.kind, causal_envelope::EntityKind::Group),
            envelope.entity.global_id.clone(),
            envelope.entity.birth_id.clone(),
        );
        if graph_identity != IdentityStatus::Conflict && checked_entities.insert(entity_key) {
            let status = identity_status(db, owner, &envelope.entity).await?;
            if status == IdentityStatus::Conflict || graph_identity == IdentityStatus::Uncontested {
                graph_identity = status;
            }
        }
        for reference in references(&envelope)? {
            edges += 1;
            if edges > MAX_OWNER_DEPENDENCIES + 2 * causal_envelope::MAX_CONTEXT_ITEMS {
                return Err(invalid());
            }
            let parent_key = event_key(&reference.event)?;
            if let Some(prior) = missing.get(&parent_key) {
                if prior.entity != reference.entity {
                    return Err(invalid());
                }
                continue;
            }
            if !envelopes.contains_key(&parent_key) {
                if let Some((parent, bytes)) = load_dependency(db, owner, &reference.event).await? {
                    loaded_bytes = loaded_bytes.checked_add(bytes).ok_or_else(invalid)?;
                    if loaded_bytes > MAX_OWNER_BYTES {
                        return Err(invalid());
                    }
                    envelopes.insert(parent_key.clone(), parent);
                } else {
                    missing.insert(parent_key, reference);
                    continue;
                }
            }
            validate_dependency(
                &envelope,
                &reference,
                envelopes.get(&parent_key).ok_or_else(invalid)?,
            )?;
            stack.push((parent_key, false));
        }
    }
    let mut missing: Vec<_> = missing.into_values().collect();
    missing.sort_by(|left, right| {
        (&left.event.writer_id, &left.event.counter)
            .cmp(&(&right.event.writer_id, &right.event.counter))
    });

    Ok((missing, graph_identity))
}

/// Caller must already have checked the opaque source and complete scoped dependencies.
/// Registration here is bookkeeping; this helper does not authorize application.
pub(crate) async fn store_checked_receipt(
    db: &mut SqliteConnection,
    user_id: i64,
    envelope: &CausalEnvelope,
    wire: &str,
) -> Result<(), DbError> {
    let dot = &envelope.event;
    let dependencies = references(envelope)?;
    let prior: Option<(i64, String, String, bool)> = sqlx::query_as("SELECT e.user_id, e.encoding, e.envelope, EXISTS(SELECT 1 FROM causal_event_receipts r WHERE r.user_id = e.user_id AND r.writer_id = e.writer_id AND r.counter = e.counter) FROM causal_events e WHERE e.writer_id = ? AND e.counter = ?")
        .bind(&dot.writer_id).bind(event_key(dot)?.1).fetch_optional(&mut *db).await?;
    if let Some((owner, encoding, existing, receipt)) = prior {
        if owner != user_id || encoding != "portable_v2" || existing != wire || !receipt {
            return Err(invalid());
        }
    } else {
        let local: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM local_causal_writers WHERE replica_id = ?) OR EXISTS(SELECT 1 FROM group_causal_replica WHERE replica_id = ?)")
            .bind(&dot.writer_id).bind(&dot.writer_id).fetch_one(&mut *db).await?;
        if local {
            return Err(invalid());
        }
        check_quota(db, user_id, wire.len(), dependencies.len()).await?;
        sqlx::query(
            "INSERT OR IGNORE INTO causal_replica_owners(user_id, replica_id) VALUES (?, ?)",
        )
        .bind(user_id)
        .bind(&dot.writer_id)
        .execute(&mut *db)
        .await?;
        sqlx::query(
            "INSERT INTO causal_events(user_id, writer_id, counter, envelope) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(&dot.writer_id)
        .bind(event_key(dot)?.1)
        .bind(wire)
        .execute(&mut *db)
        .await?;
        sqlx::query(
            "INSERT INTO causal_event_receipts(user_id, writer_id, counter) VALUES (?, ?, ?)",
        )
        .bind(user_id)
        .bind(&dot.writer_id)
        .bind(event_key(dot)?.1)
        .execute(&mut *db)
        .await?;
        for parent in &dependencies {
            sqlx::query("INSERT INTO causal_receipt_dependencies(user_id, writer_id, counter, dependency_writer_id, dependency_counter) VALUES (?, ?, ?, ?, ?)")
                .bind(user_id).bind(&dot.writer_id).bind(event_key(dot)?.1).bind(&parent.event.writer_id).bind(event_key(&parent.event)?.1).execute(&mut *db).await?;
        }
    }
    Ok(())
}
