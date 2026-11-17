use sqlx::SqliteConnection;

use crate::{DbError, db_context::DbContext, model::user::User};

mod local_journal;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EventId {
    pub replica_id: String,
    pub counter: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroupRenameEvent {
    pub id: EventId,
    pub group_global_id: String,
    pub parents: Vec<EventId>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReplicaPrincipal {
    pub owner_user_id: i64,
    pub replica_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestResult {
    AcceptedReady,
    AcceptedPending { missing: Vec<EventId> },
    AlreadyPresent,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReceivedGroupRename {
    pub event: GroupRenameEvent,
    pub missing: Vec<EventId>,
}

pub const MAX_RENAME_EVENT_PARENTS: usize = 1024;
pub const MAX_RENAME_EVENT_NAME_BYTES: usize = 65536;

pub async fn ingest_group_rename(
    db: &DbContext,
    principal: &ReplicaPrincipal,
    event: &GroupRenameEvent,
) -> Result<IngestResult, DbError> {
    let mut parents = event.parents.clone();
    parents.sort_by(|left, right| {
        (&left.replica_id, left.counter).cmp(&(&right.replica_id, right.counter))
    });
    if principal.replica_id.is_empty()
        || principal.replica_id.len() > 128
        || event.id.replica_id != principal.replica_id
        || event.id.counter <= 0
        || event.group_global_id.is_empty()
        || event.group_global_id.len() > 128
        || event.name.len() > MAX_RENAME_EVENT_NAME_BYTES
        || parents.len() > MAX_RENAME_EVENT_PARENTS
        || parents.iter().any(|id| {
            id.replica_id.is_empty()
                || id.replica_id.len() > 128
                || id.counter <= 0
                || *id == event.id
                || (id.replica_id == event.id.replica_id && id.counter >= event.id.counter)
        })
        || parents.windows(2).any(|pair| pair[0] == pair[1])
    {
        return Err(invalid_ingestion());
    }
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM causal_replica_owners WHERE user_id = ? AND replica_id = ?) AND NOT EXISTS(SELECT 1 FROM group_causal_replica WHERE singleton = 1 AND replica_id = ?) AND NOT EXISTS(SELECT 1 FROM local_causal_writers WHERE replica_id = ?)")
        .bind(principal.owner_user_id).bind(&principal.replica_id).bind(&principal.replica_id).bind(&principal.replica_id).fetch_one(&mut *transaction).await?;
    let group_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?",
    )
    .bind(principal.owner_user_id)
    .bind(&event.group_global_id)
    .fetch_one(&mut *transaction)
    .await?;
    let generation: Option<i64> = sqlx::query_scalar(
        "SELECT generation FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?",
    )
    .bind(principal.owner_user_id)
    .bind(&event.group_global_id)
    .fetch_optional(&mut *transaction)
    .await?;
    if !allowed || generation.is_none() || group_count > 1 {
        return Err(invalid_ingestion());
    }
    let prior: Option<(i64, String, String)> = sqlx::query_as("SELECT user_id, group_global_id, name FROM received_group_renames WHERE replica_id = ? AND counter = ?")
        .bind(&event.id.replica_id).bind(event.id.counter).fetch_optional(&mut *transaction).await?;
    if let Some((owner, global, name)) = prior {
        let prior_parents = received_parents(&mut transaction, &event.id).await?;
        if owner != principal.owner_user_id
            || global != event.group_global_id
            || name != event.name
            || prior_parents != parents
        {
            return Err(invalid_ingestion());
        }
        transaction.commit().await?;

        return Ok(IngestResult::AlreadyPresent);
    }
    if generation != Some(1) {
        return Err(invalid_ingestion());
    }
    let collision: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_causal_events WHERE replica_id = ? AND counter = ?) OR EXISTS(SELECT 1 FROM membership_causal_events WHERE replica_id = ? AND counter = ?) OR EXISTS(SELECT 1 FROM causal_events WHERE writer_id = ? AND counter = ?)")
        .bind(&event.id.replica_id).bind(event.id.counter).bind(&event.id.replica_id).bind(event.id.counter).bind(&event.id.replica_id).bind(event.id.counter).fetch_one(&mut *transaction).await?;
    if collision {
        return Err(invalid_ingestion());
    }
    for parent in &parents {
        validate_parent(
            &mut transaction,
            principal.owner_user_id,
            &event.group_global_id,
            &event.id,
            parent,
        )
        .await?;
    }
    sqlx::query("INSERT INTO received_group_renames (replica_id, counter, user_id, group_global_id, name) VALUES (?, ?, ?, ?, ?)")
        .bind(&event.id.replica_id).bind(event.id.counter).bind(principal.owner_user_id).bind(&event.group_global_id).bind(&event.name).execute(&mut *transaction).await?;
    for parent in &parents {
        sqlx::query("INSERT INTO received_group_rename_parents (replica_id, counter, parent_replica_id, parent_counter) VALUES (?, ?, ?, ?)")
            .bind(&event.id.replica_id).bind(event.id.counter).bind(&parent.replica_id).bind(parent.counter).execute(&mut *transaction).await?;
    }
    let missing = missing_ancestors(
        &mut transaction,
        principal.owner_user_id,
        &event.group_global_id,
        &parents,
    )
    .await?;
    transaction.commit().await?;

    if missing.is_empty() {
        Ok(IngestResult::AcceptedReady)
    } else {
        Ok(IngestResult::AcceptedPending { missing })
    }
}

pub async fn get_received_group_renames(
    db: &DbContext,
    user: &User,
    global: &str,
) -> Result<Vec<ReceivedGroupRename>, DbError> {
    let mut transaction = db.begin().await?;
    let rows: Vec<(String, i64, String)> = sqlx::query_as("SELECT replica_id, counter, name FROM received_group_renames WHERE user_id = ? AND group_global_id = ? ORDER BY replica_id, counter")
        .bind(user.id).bind(global).fetch_all(&mut *transaction).await?;
    let mut events = Vec::with_capacity(rows.len());
    for (replica_id, counter, name) in rows {
        let id = EventId {
            replica_id,
            counter,
        };
        let parents = received_parents(&mut transaction, &id).await?;
        let missing = missing_ancestors(&mut transaction, user.id, global, &parents).await?;
        events.push(ReceivedGroupRename {
            event: GroupRenameEvent {
                id,
                group_global_id: global.to_owned(),
                parents,
                name,
            },
            missing,
        });
    }
    transaction.commit().await?;
    Ok(events)
}

fn invalid_ingestion() -> DbError {
    DbError::Protocol("Invalid causal rename event".into())
}

async fn received_parents(
    db: &mut SqliteConnection,
    id: &EventId,
) -> Result<Vec<EventId>, DbError> {
    let rows: Vec<(String, i64)> = sqlx::query_as("SELECT parent_replica_id, parent_counter FROM received_group_rename_parents WHERE replica_id = ? AND counter = ? ORDER BY parent_replica_id, parent_counter")
        .bind(&id.replica_id).bind(id.counter).fetch_all(db).await?;

    Ok(event_ids(rows))
}

async fn validate_parent(
    db: &mut SqliteConnection,
    owner: i64,
    global: &str,
    child: &EventId,
    parent: &EventId,
) -> Result<(), DbError> {
    let locations: Vec<(i64, String)> = sqlx::query_as("SELECT user_id, group_global_id FROM group_causal_events WHERE replica_id = ? AND counter = ? UNION ALL SELECT user_id, group_global_id FROM received_group_renames WHERE replica_id = ? AND counter = ?")
        .bind(&parent.replica_id).bind(parent.counter).bind(&parent.replica_id).bind(parent.counter).fetch_all(&mut *db).await?;
    if locations.len() > 1
        || locations
            .iter()
            .any(|(user, group)| *user != owner || group != global)
    {
        return Err(invalid_ingestion());
    }
    let membership: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM membership_causal_events WHERE replica_id = ? AND counter = ?)")
        .bind(&parent.replica_id).bind(parent.counter).fetch_one(&mut *db).await?;
    if membership {
        return Err(invalid_ingestion());
    }
    let cycle: bool = sqlx::query_scalar("WITH RECURSIVE ancestors(replica_id, counter) AS (SELECT ?1, ?2 UNION SELECT p.parent_replica_id, p.parent_counter FROM received_group_rename_parents p JOIN ancestors a ON p.replica_id = a.replica_id AND p.counter = a.counter) SELECT EXISTS(SELECT 1 FROM ancestors WHERE replica_id = ?3 AND counter = ?4)")
        .bind(&parent.replica_id).bind(parent.counter).bind(&child.replica_id).bind(child.counter).fetch_one(db).await?;
    if cycle {
        return Err(invalid_ingestion());
    }

    Ok(())
}

async fn missing_ancestors(
    db: &mut SqliteConnection,
    owner: i64,
    global: &str,
    parents: &[EventId],
) -> Result<Vec<EventId>, DbError> {
    let mut pending = parents.to_vec();
    let mut visited = std::collections::HashSet::new();
    let mut missing = Vec::new();
    while let Some(id) = pending.pop() {
        if !visited.insert((id.replica_id.clone(), id.counter)) {
            continue;
        }
        if visited.len() > 100_000 {
            return Err(invalid_ingestion());
        }
        let applied: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_causal_events WHERE user_id = ? AND group_global_id = ? AND replica_id = ? AND counter = ?)")
            .bind(owner).bind(global).bind(&id.replica_id).bind(id.counter).fetch_one(&mut *db).await?;
        if applied {
            continue;
        }
        let received: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM received_group_renames WHERE user_id = ? AND group_global_id = ? AND replica_id = ? AND counter = ?)")
            .bind(owner).bind(global).bind(&id.replica_id).bind(id.counter).fetch_one(&mut *db).await?;
        if received {
            pending.extend(received_parents(&mut *db, &id).await?);
        } else {
            missing.push(id);
        }
    }
    missing.sort_by(|left, right| {
        (&left.replica_id, left.counter).cmp(&(&right.replica_id, right.counter))
    });
    Ok(missing)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HistoryOrigin {
    UnknownPreUpgrade,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroupHistory {
    pub group_global_id: String,
    pub ancestry: HistoryOrigin,
    pub frontier: Vec<EventId>,
    pub events: Vec<GroupRenameEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GroupLifecycleAction {
    Delete {
        deletion_operation_id: String,
    },
    Restore {
        deletion_operation_id: String,
        deletion_event: Option<EventId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroupLifecycleEvent {
    pub id: EventId,
    pub parents: Vec<EventId>,
    pub action: GroupLifecycleAction,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MembershipAction {
    Move {
        from_group_global_id: Option<String>,
        to_group_global_id: Option<String>,
    },
    Detach {
        group_global_id: String,
        deletion_operation_id: String,
        deletion_event: EventId,
    },
    Restore {
        group_global_id: String,
        deletion_operation_id: String,
        deletion_event: Option<EventId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MembershipEvent {
    pub id: EventId,
    pub parents: Vec<EventId>,
    pub action: MembershipAction,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MembershipHistory {
    pub ancestry: HistoryOrigin,
    pub frontier: Vec<EventId>,
    pub events: Vec<MembershipEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroupMemberAddedEvent {
    pub id: EventId,
    pub parents: Vec<EventId>,
    pub model_global_id: String,
    pub model_incarnation: String,
    pub membership_event: EventId,
}

pub async fn get_group_member_added_history(
    db: &DbContext,
    user: &User,
    global: &str,
) -> Result<Option<Vec<GroupMemberAddedEvent>>, DbError> {
    let mut transaction = db.begin().await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?)",
    )
    .bind(user.id)
    .bind(global)
    .fetch_one(&mut *transaction)
    .await?;
    if !exists {
        return Ok(None);
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?",
    )
    .bind(user.id)
    .bind(global)
    .fetch_one(&mut *transaction)
    .await?;
    if global.is_empty() || count > 1 {
        return Err(DbError::Protocol(
            "Invalid or ambiguous group sync identity".into(),
        ));
    }
    let rows: Vec<MemberAddedRow> = sqlx::query_as("SELECT event_id, replica_id, counter, model_global_id, model_incarnation, membership_replica_id, membership_counter FROM group_causal_event_facts WHERE user_id = ? AND group_global_id = ? AND event_kind = 'member_added' ORDER BY event_id")
        .bind(user.id).bind(global).fetch_all(&mut *transaction).await?;
    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let parents = group_parents(&mut transaction, row.event_id).await?;
        events.push(GroupMemberAddedEvent {
            id: EventId {
                replica_id: row.replica_id,
                counter: row.counter,
            },
            parents,
            model_global_id: row.model_global_id,
            model_incarnation: row.model_incarnation,
            membership_event: EventId {
                replica_id: row.membership_replica_id,
                counter: row.membership_counter,
            },
        });
    }
    transaction.commit().await?;
    Ok(Some(events))
}

#[derive(sqlx::FromRow)]
struct MemberAddedRow {
    event_id: i64,
    replica_id: String,
    counter: i64,
    model_global_id: String,
    model_incarnation: String,
    membership_replica_id: String,
    membership_counter: i64,
}

pub async fn get_group_lifecycle_history(
    db: &DbContext,
    user: &User,
    global: &str,
) -> Result<Option<Vec<GroupLifecycleEvent>>, DbError> {
    let mut transaction = db.begin().await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?)",
    )
    .bind(user.id)
    .bind(global)
    .fetch_one(&mut *transaction)
    .await?;
    if !exists {
        return Ok(None);
    }
    let duplicates: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?",
    )
    .bind(user.id)
    .bind(global)
    .fetch_one(&mut *transaction)
    .await?;
    if global.is_empty() || duplicates > 1 {
        return Err(DbError::Protocol(
            "Invalid or ambiguous group sync identity".into(),
        ));
    }
    let rows: Vec<LifecycleRow> = sqlx::query_as("SELECT event_id, replica_id, counter, event_kind, deletion_operation_id, deletion_replica_id, deletion_counter FROM group_causal_event_facts WHERE user_id = ? AND group_global_id = ? AND event_kind IN ('delete', 'restore') ORDER BY event_id")
        .bind(user.id).bind(global).fetch_all(&mut *transaction).await?;
    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let parents = group_parents(&mut transaction, row.event_id).await?;
        let deletion_event = optional_event(row.deletion_replica_id, row.deletion_counter)?;
        let action = match row.event_kind.as_str() {
            "delete" => GroupLifecycleAction::Delete {
                deletion_operation_id: row.deletion_operation_id,
            },
            "restore" => GroupLifecycleAction::Restore {
                deletion_operation_id: row.deletion_operation_id,
                deletion_event,
            },
            _ => return Err(DbError::Protocol("Invalid lifecycle event".into())),
        };
        events.push(GroupLifecycleEvent {
            id: EventId {
                replica_id: row.replica_id,
                counter: row.counter,
            },
            parents,
            action,
        });
    }
    transaction.commit().await?;
    Ok(Some(events))
}

pub async fn get_membership_history(
    db: &DbContext,
    user: &User,
    global: &str,
    incarnation: &str,
) -> Result<Option<MembershipHistory>, DbError> {
    let mut transaction = db.begin().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models WHERE model_user_id = ? AND model_unique_global_id = ? AND model_incarnation = ?) OR EXISTS(SELECT 1 FROM membership_causal_events WHERE user_id = ? AND model_global_id = ? AND model_incarnation = ?)")
        .bind(user.id).bind(global).bind(incarnation).bind(user.id).bind(global).bind(incarnation).fetch_one(&mut *transaction).await?;
    if !exists {
        return Ok(None);
    }
    if global.is_empty() || incarnation.is_empty() {
        return Err(DbError::Protocol("Invalid model sync identity".into()));
    }
    let rows: Vec<MembershipRow> = sqlx::query_as("SELECT event_id, replica_id, counter, event_kind, group_global_id, deletion_operation_id, deletion_replica_id, deletion_counter, from_group_global_id, to_group_global_id FROM membership_causal_event_facts WHERE user_id = ? AND model_global_id = ? AND model_incarnation = ? ORDER BY event_id")
        .bind(user.id).bind(global).bind(incarnation).fetch_all(&mut *transaction).await?;
    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let parents: Vec<(String, i64)> = sqlx::query_as("SELECT p.replica_id, p.counter FROM membership_causal_parents l JOIN membership_causal_events p ON p.event_id = l.parent_id WHERE l.event_id = ? ORDER BY p.replica_id, p.counter")
            .bind(row.event_id).fetch_all(&mut *transaction).await?;
        let deletion_event = optional_event(row.deletion_replica_id, row.deletion_counter)?;
        let action = match row.event_kind.as_str() {
            "move" => MembershipAction::Move {
                from_group_global_id: row.from_group_global_id,
                to_group_global_id: row.to_group_global_id,
            },
            "detach" => MembershipAction::Detach {
                group_global_id: row.group_global_id,
                deletion_operation_id: row.deletion_operation_id,
                deletion_event: deletion_event
                    .ok_or_else(|| DbError::Protocol("Missing deletion event".into()))?,
            },
            "restore" => MembershipAction::Restore {
                group_global_id: row.group_global_id,
                deletion_operation_id: row.deletion_operation_id,
                deletion_event,
            },
            _ => return Err(DbError::Protocol("Invalid membership event".into())),
        };
        events.push(MembershipEvent {
            id: EventId {
                replica_id: row.replica_id,
                counter: row.counter,
            },
            parents: event_ids(parents),
            action,
        });
    }
    let frontier: Vec<(String, i64)> = sqlx::query_as("SELECT e.replica_id, e.counter FROM membership_causal_frontier f JOIN membership_causal_events e ON e.event_id = f.event_id WHERE f.user_id = ? AND f.model_global_id = ? AND f.model_incarnation = ? ORDER BY e.replica_id, e.counter")
        .bind(user.id).bind(global).bind(incarnation).fetch_all(&mut *transaction).await?;
    transaction.commit().await?;

    Ok(Some(MembershipHistory {
        ancestry: HistoryOrigin::UnknownPreUpgrade,
        frontier: event_ids(frontier),
        events,
    }))
}

#[derive(sqlx::FromRow)]
struct LifecycleRow {
    event_id: i64,
    replica_id: String,
    counter: i64,
    event_kind: String,
    deletion_operation_id: String,
    deletion_replica_id: Option<String>,
    deletion_counter: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct MembershipRow {
    event_id: i64,
    replica_id: String,
    counter: i64,
    event_kind: String,
    group_global_id: String,
    deletion_operation_id: String,
    deletion_replica_id: Option<String>,
    deletion_counter: Option<i64>,
    from_group_global_id: Option<String>,
    to_group_global_id: Option<String>,
}

fn event_ids(rows: Vec<(String, i64)>) -> Vec<EventId> {
    rows.into_iter()
        .map(|(replica_id, counter)| EventId {
            replica_id,
            counter,
        })
        .collect()
}

fn optional_event(
    replica_id: Option<String>,
    counter: Option<i64>,
) -> Result<Option<EventId>, DbError> {
    match (replica_id, counter) {
        (Some(replica_id), Some(counter)) => Ok(Some(EventId {
            replica_id,
            counter,
        })),
        (None, None) => Ok(None),
        _ => Err(DbError::Protocol("Invalid deletion event".into())),
    }
}

async fn group_parents(db: &mut SqliteConnection, event: i64) -> Result<Vec<EventId>, DbError> {
    let rows = sqlx::query_as("SELECT p.replica_id, p.counter FROM group_causal_parents l JOIN group_causal_events p ON p.event_id = l.parent_id WHERE l.event_id = ? ORDER BY p.replica_id, p.counter")
        .bind(event).fetch_all(db).await?;

    Ok(event_ids(rows))
}

#[derive(sqlx::FromRow)]
struct EventRow {
    event_id: i64,
    replica_id: String,
    counter: i64,
    name: String,
}

pub async fn get_group_history(
    db: &DbContext,
    user: &User,
    global: &str,
) -> Result<Option<GroupHistory>, DbError> {
    let mut transaction = db.begin().await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM group_sync_heads WHERE user_id = ? AND group_global_id = ?)",
    )
    .bind(user.id)
    .bind(global)
    .fetch_one(&mut *transaction)
    .await?;
    if !exists {
        transaction.commit().await?;

        return Ok(None);
    }
    let group_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?",
    )
    .bind(user.id)
    .bind(global)
    .fetch_one(&mut *transaction)
    .await?;
    if global.is_empty() || group_count > 1 {
        return Err(DbError::Protocol(
            "Invalid or ambiguous group sync identity".into(),
        ));
    }
    let rows: Vec<EventRow> = sqlx::query_as("SELECT event_id, replica_id, counter, name FROM group_causal_event_facts WHERE user_id = ? AND group_global_id = ? AND event_kind = 'rename' ORDER BY event_id")
        .bind(user.id).bind(global).fetch_all(&mut *transaction).await?;
    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let parents: Vec<(String, i64)> = sqlx::query_as("SELECT p.replica_id, p.counter FROM group_causal_parents link JOIN group_causal_events p ON p.event_id = link.parent_id WHERE link.event_id = ? ORDER BY p.replica_id, p.counter")
            .bind(row.event_id).fetch_all(&mut *transaction).await?;
        events.push(GroupRenameEvent {
            id: EventId {
                replica_id: row.replica_id,
                counter: row.counter,
            },
            group_global_id: global.to_owned(),
            parents: parents
                .into_iter()
                .map(|(replica_id, counter)| EventId {
                    replica_id,
                    counter,
                })
                .collect(),
            name: row.name,
        });
    }
    let frontier: Vec<(String, i64)> = sqlx::query_as("SELECT e.replica_id, e.counter FROM group_causal_frontier f JOIN group_causal_events e ON e.event_id = f.event_id WHERE f.user_id = ? AND f.group_global_id = ? ORDER BY e.replica_id, e.counter")
        .bind(user.id).bind(global).fetch_all(&mut *transaction).await?;
    transaction.commit().await?;

    Ok(Some(GroupHistory {
        group_global_id: global.to_owned(),
        ancestry: HistoryOrigin::UnknownPreUpgrade,
        frontier: frontier
            .into_iter()
            .map(|(replica_id, counter)| EventId {
                replica_id,
                counter,
            })
            .collect(),
        events,
    }))
}

pub(crate) async fn record_rename(
    db: &mut SqliteConnection,
    user: &User,
    global: &str,
    name: &str,
) -> Result<(), DbError> {
    local_journal::record_group(
        db,
        user,
        global,
        &local_journal::GroupFact {
            kind: "rename",
            name,
            ..Default::default()
        },
    )
    .await?;

    Ok(())
}

pub(crate) async fn find_deletion_event(
    db: &mut SqliteConnection,
    user: &User,
    global: &str,
    operation: &str,
) -> Result<Option<EventId>, DbError> {
    let row: Option<(String, i64)> = sqlx::query_as("SELECT replica_id, counter FROM group_causal_event_facts WHERE user_id = ? AND group_global_id = ? AND event_kind = 'delete' AND deletion_operation_id = ?")
        .bind(user.id).bind(global).bind(operation).fetch_optional(db).await?;

    Ok(row.map(|(replica_id, counter)| EventId {
        replica_id,
        counter,
    }))
}

pub(crate) async fn record_lifecycle(
    db: &mut SqliteConnection,
    user: &User,
    global: &str,
    operation: &str,
    deleting: bool,
    deletion_event: Option<&EventId>,
) -> Result<EventId, DbError> {
    local_journal::record_group(
        db,
        user,
        global,
        &local_journal::GroupFact {
            kind: if deleting { "delete" } else { "restore" },
            deletion_operation_id: Some(operation),
            deletion_replica_id: deletion_event.map(|event| event.replica_id.as_str()),
            deletion_counter: deletion_event.map(|event| event.counter),
            ..Default::default()
        },
    )
    .await
}

pub(crate) async fn record_membership(
    db: &mut SqliteConnection,
    user: &User,
    model: &str,
    incarnation: &str,
    action: &MembershipAction,
) -> Result<EventId, DbError> {
    let (kind, global, operation, deletion_event, from, to) = match action {
        MembershipAction::Move {
            from_group_global_id,
            to_group_global_id,
        } => (
            "move",
            to_group_global_id
                .as_deref()
                .or(from_group_global_id.as_deref())
                .unwrap_or(""),
            "",
            None,
            from_group_global_id.as_deref(),
            to_group_global_id.as_deref(),
        ),
        MembershipAction::Detach {
            group_global_id,
            deletion_operation_id,
            deletion_event,
        } => (
            "detach",
            group_global_id.as_str(),
            deletion_operation_id.as_str(),
            Some(deletion_event),
            None,
            None,
        ),
        MembershipAction::Restore {
            group_global_id,
            deletion_operation_id,
            deletion_event,
        } => (
            "restore",
            group_global_id.as_str(),
            deletion_operation_id.as_str(),
            deletion_event.as_ref(),
            None,
            None,
        ),
    };

    local_journal::record_membership(
        db,
        user,
        model,
        incarnation,
        &local_journal::MembershipFact {
            kind,
            group_global_id: global,
            deletion_operation_id: operation,
            deletion_replica_id: deletion_event.map(|event| event.replica_id.as_str()),
            deletion_counter: deletion_event.map(|event| event.counter),
            from_group_global_id: from,
            to_group_global_id: to,
        },
    )
    .await
}

pub(crate) async fn record_group_member_added(
    db: &mut SqliteConnection,
    user: &User,
    group: &str,
    model: &str,
    incarnation: &str,
    membership: &EventId,
) -> Result<(), DbError> {
    local_journal::record_group(
        db,
        user,
        group,
        &local_journal::GroupFact {
            kind: "member_added",
            model_global_id: Some(model),
            model_incarnation: Some(incarnation),
            membership_replica_id: Some(&membership.replica_id),
            membership_counter: Some(membership.counter),
            ..Default::default()
        },
    )
    .await?;

    Ok(())
}
