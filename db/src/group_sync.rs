use std::{collections::HashMap, fmt::Write};

use sqlx::SqliteConnection;

use crate::{DbError, db_context::DbContext, model::user::User, time_now};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AssignmentChange {
    pub model_global_id: String,
    pub expected_incarnation: String,
    pub expected_group_global_id: Option<String>,
    pub expected_revision: i64,
    pub expected_lifecycle_revision: i64,
    pub expected_deleted_at: Option<String>,
    pub target_group_global_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroupPrecondition {
    pub group_global_id: String,
    pub expected_generation: i64,
    pub expected_head: i64,
    pub expected_state: String,
    pub expected_revision: Option<i64>,
    pub expected_deleted_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentBatchResult {
    Applied,
    AlreadyApplied,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum KeepResource {
    None,
    Existing {
        resource_global_id: String,
        expected_incarnation: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GroupLifecycleChange {
    Delete {
        group_global_id: String,
        deletion_operation_id: String,
    },
    Restore {
        group_global_id: String,
        deletion_operation_id: String,
    },
    Keep {
        group_global_id: String,
        deletion_operation_id: String,
        name: String,
        expected_birth_id: Option<String>,
        resource: KeepResource,
    },
}

impl GroupLifecycleChange {
    fn group_id(&self) -> &str {
        match self {
            Self::Delete {
                group_global_id, ..
            }
            | Self::Restore {
                group_global_id, ..
            }
            | Self::Keep {
                group_global_id, ..
            } => group_global_id,
        }
    }
    fn deletion_id(&self) -> &str {
        match self {
            Self::Delete {
                deletion_operation_id,
                ..
            }
            | Self::Restore {
                deletion_operation_id,
                ..
            }
            | Self::Keep {
                deletion_operation_id,
                ..
            } => deletion_operation_id,
        }
    }
}

type AssignmentRow = (i64, Option<String>, i64, Option<String>, bool, String, i64);
type GroupRow = (Option<i64>, Option<i64>, Option<String>, i64, i64, String);
type GroupState = (Option<i64>, String);

pub async fn apply_assignment_batch(
    db: &DbContext,
    user: &User,
    operation_id: &str,
    changes: &[AssignmentChange],
    groups: &[GroupPrecondition],
) -> Result<AssignmentBatchResult, DbError> {
    apply_group_sync_batch(db, user, operation_id, changes, groups, &[], 0).await
}

pub async fn apply_group_sync_batch(
    db: &DbContext,
    user: &User,
    operation_id: &str,
    changes: &[AssignmentChange],
    groups: &[GroupPrecondition],
    lifecycle: &[GroupLifecycleChange],
    retention_days: u32,
) -> Result<AssignmentBatchResult, DbError> {
    // This internal prototype does not supply original causal envelopes. Its direct
    // writes remain an incomplete-history boundary until the authenticated reducer
    // applies events atomically; minting local user actions here would invent authorship.
    let mut changes: Vec<_> = changes.iter().collect();
    changes.sort_by(|a, b| a.model_global_id.cmp(&b.model_global_id));
    let mut groups: Vec<_> = groups.iter().collect();
    groups.sort_by(|a, b| a.group_global_id.cmp(&b.group_global_id));
    let mut lifecycle: Vec<_> = lifecycle.iter().collect();
    lifecycle.sort_by_key(|action| action.group_id());
    validate_batch_shape(operation_id, &changes, &groups, &lifecycle)?;
    let payload = canonical_payload(&changes, &groups, &lifecycle, retention_days);
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let prior: Option<String> = sqlx::query_scalar(
        "SELECT payload FROM assignment_batch_journal WHERE user_id = ? AND operation_id = ?",
    )
    .bind(user.id)
    .bind(operation_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(prior) = prior {
        if prior != payload {
            return Err(invalid_batch());
        }
        tx.commit().await?;
        return Ok(AssignmentBatchResult::AlreadyApplied);
    }
    let mut group_states = validate_groups(&mut tx, user, &groups).await?;
    let updates = validate_assignments(&mut tx, user, &changes, &group_states, &lifecycle).await?;
    validate_lifecycle(
        &mut tx,
        user,
        &changes,
        &group_states,
        &lifecycle,
        retention_days,
    )
    .await?;
    apply_lifecycle(&mut tx, user, &mut group_states, &lifecycle).await?;
    for (id, target) in updates {
        let target_id = match target {
            Some(global) => Some(
                group_states
                    .get(global)
                    .and_then(|(id, state)| (*id).filter(|_| state == "active"))
                    .ok_or_else(invalid_batch)?,
            ),
            None => None,
        };
        sqlx::query(
            "UPDATE models SET model_group_id = ? WHERE model_id = ? AND model_user_id = ?",
        )
        .bind(target_id)
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    }
    for action in &lifecycle {
        if let GroupLifecycleChange::Keep {
            resource: KeepResource::Existing { .. },
            ..
        } = action
        {
            let group_id = group_states[action.group_id()]
                .0
                .ok_or_else(invalid_batch)?;
            let has_models: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models WHERE model_user_id = ? AND model_group_id = ?)")
                .bind(user.id).bind(group_id).fetch_one(&mut *tx).await?;
            if !has_models {
                return Err(invalid_batch());
            }
        }
    }
    for action in &lifecycle {
        if let GroupLifecycleChange::Delete {
            group_global_id,
            deletion_operation_id,
        } = action
        {
            let id = group_states[group_global_id.as_str()]
                .0
                .ok_or_else(invalid_batch)?;
            sqlx::query("INSERT INTO group_deletion_records (user_id, group_global_id, operation_id, deleted_at, group_revision, group_birth_id) SELECT ?, ?, ?, group_deleted_at, group_revision, group_birth_id FROM models_group WHERE group_id = ? AND group_user_id = ?")
                .bind(user.id).bind(group_global_id).bind(deletion_operation_id).bind(id).bind(user.id).execute(&mut *tx).await?;
        }
    }
    sqlx::query(
        "INSERT INTO assignment_batch_journal (user_id, operation_id, payload) VALUES (?, ?, ?)",
    )
    .bind(user.id)
    .bind(operation_id)
    .bind(payload)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(AssignmentBatchResult::Applied)
}

async fn validate_groups<'a>(
    db: &mut SqliteConnection,
    user: &User,
    groups: &[&'a GroupPrecondition],
) -> Result<HashMap<&'a str, GroupState>, DbError> {
    let mut result = HashMap::new();
    for group in groups {
        let rows: Vec<GroupRow> = sqlx::query_as("SELECT g.group_id, g.group_revision, g.group_deleted_at, h.generation, h.head, h.state FROM group_sync_heads h LEFT JOIN models_group g ON g.group_user_id = h.user_id AND g.group_unique_global_id = h.group_global_id WHERE h.user_id = ? AND h.group_global_id = ?")
            .bind(user.id).bind(&group.group_global_id).fetch_all(&mut *db).await?;
        let [(id, revision, deleted_at, generation, head, state)] = rows.as_slice() else {
            return Err(invalid_batch());
        };
        let actual_state = if id.is_none() {
            "absent"
        } else if deleted_at.is_some() {
            "trashed"
        } else {
            "active"
        };
        if *revision != group.expected_revision
            || *deleted_at != group.expected_deleted_at
            || *generation != group.expected_generation
            || *head != group.expected_head
            || *state != group.expected_state
            || state != actual_state
        {
            return Err(invalid_batch());
        }
        result.insert(group.group_global_id.as_str(), (*id, state.clone()));
    }
    Ok(result)
}

async fn validate_assignments<'a>(
    db: &mut SqliteConnection,
    user: &User,
    changes: &[&'a AssignmentChange],
    groups: &HashMap<&str, GroupState>,
    lifecycle: &[&GroupLifecycleChange],
) -> Result<Vec<(i64, Option<&'a str>)>, DbError> {
    let mut updates = Vec::new();
    for change in changes {
        for id in change
            .expected_group_global_id
            .iter()
            .chain(change.target_group_global_id.iter())
        {
            if !groups.contains_key(id.as_str()) {
                return Err(invalid_batch());
            }
        }
        let rows: Vec<AssignmentRow> = sqlx::query_as("SELECT m.model_id, g.group_unique_global_id, m.model_assignment_revision, m.model_deleted, (m.model_group_id IS NOT NULL AND g.group_id IS NULL), m.model_incarnation, m.model_lifecycle_revision FROM models m LEFT JOIN models_group g ON g.group_id = m.model_group_id AND g.group_user_id = m.model_user_id WHERE m.model_user_id = ? AND m.model_unique_global_id = ?")
            .bind(user.id).bind(&change.model_global_id).fetch_all(&mut *db).await?;
        let [(id, current, revision, deleted_at, invalid_group, incarnation, lifecycle_revision)] =
            rows.as_slice()
        else {
            return Err(invalid_batch());
        };
        if *invalid_group
            || incarnation.is_empty()
            || *incarnation != change.expected_incarnation
            || *current != change.expected_group_global_id
            || *revision != change.expected_revision
            || *lifecycle_revision != change.expected_lifecycle_revision
            || *deleted_at != change.expected_deleted_at
        {
            return Err(invalid_batch());
        }
        if let Some(target) = change.target_group_global_id.as_deref() {
            let action = lifecycle.iter().find(|action| action.group_id() == target);
            let active = match action {
                Some(GroupLifecycleChange::Delete { .. }) => false,
                Some(GroupLifecycleChange::Restore { .. } | GroupLifecycleChange::Keep { .. }) => {
                    true
                }
                None => groups
                    .get(target)
                    .is_some_and(|(_, state)| state == "active"),
            };
            if !active {
                return Err(invalid_batch());
            }
        }
        if *current != change.target_group_global_id {
            updates.push((*id, change.target_group_global_id.as_deref()));
        }
    }
    Ok(updates)
}

async fn validate_lifecycle(
    db: &mut SqliteConnection,
    user: &User,
    changes: &[&AssignmentChange],
    groups: &HashMap<&str, GroupState>,
    lifecycle: &[&GroupLifecycleChange],
    retention_days: u32,
) -> Result<(), DbError> {
    for action in lifecycle {
        let (id, state) = groups.get(action.group_id()).ok_or_else(invalid_batch)?;
        match action {
            GroupLifecycleChange::Delete { .. } => {
                let id = id.ok_or_else(invalid_batch)?;
                if state != "active" {
                    return Err(invalid_batch());
                }
                let members: Vec<String> = sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_user_id = ? AND model_group_id = ?")
                    .bind(user.id).bind(id).fetch_all(&mut *db).await?;
                if members.iter().any(|member| {
                    !changes.iter().any(|change| {
                        change.model_global_id == *member
                            && change.expected_group_global_id.as_deref() == Some(action.group_id())
                            && change.target_group_global_id.as_deref() != Some(action.group_id())
                    })
                }) {
                    return Err(invalid_batch());
                }
            }
            GroupLifecycleChange::Restore { .. } | GroupLifecycleChange::Keep { .. } => {
                if !matches!(state.as_str(), "trashed" | "absent") {
                    return Err(invalid_batch());
                }
                let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_deletion_records r JOIN group_sync_heads h ON h.user_id = r.user_id AND h.group_global_id = r.group_global_id AND h.last_deletion_operation_id = r.operation_id WHERE r.user_id = ? AND r.group_global_id = ? AND r.operation_id = ? AND r.restored_at IS NULL)")
                    .bind(user.id).bind(action.group_id()).bind(action.deletion_id()).fetch_one(&mut *db).await?;
                if !valid {
                    return Err(invalid_batch());
                }
                if let GroupLifecycleChange::Keep {
                    expected_birth_id,
                    resource,
                    ..
                } = action
                {
                    resolve_keep_resource(db, user, resource).await?;
                    let (recorded_birth,): (Option<String>,) = sqlx::query_as("SELECT group_birth_id FROM group_deletion_records WHERE user_id = ? AND group_global_id = ? AND operation_id = ? AND restored_at IS NULL")
                        .bind(user.id).bind(action.group_id()).bind(action.deletion_id()).fetch_one(&mut *db).await?;
                    if recorded_birth != *expected_birth_id {
                        return Err(invalid_batch());
                    }
                    if let Some(birth) = expected_birth_id {
                        let registered: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entity_births WHERE user_id = ? AND entity_kind = 'group' AND global_id = ? AND birth_id = ?)")
                            .bind(user.id).bind(action.group_id()).bind(birth).fetch_one(&mut *db).await?;
                        if !registered {
                            return Err(invalid_batch());
                        }
                    } else if id.is_none() {
                        let known_birth: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entity_births WHERE user_id = ? AND entity_kind = 'group' AND global_id = ?)")
                            .bind(user.id).bind(action.group_id()).fetch_one(&mut *db).await?;
                        if known_birth {
                            return Err(invalid_batch());
                        }
                    }
                    if let Some(id) = id {
                        let (current_birth,): (Option<String>,) = sqlx::query_as("SELECT group_birth_id FROM models_group WHERE group_id = ? AND group_user_id = ?")
                            .bind(id).bind(user.id).fetch_one(&mut *db).await?;
                        if current_birth != *expected_birth_id {
                            return Err(invalid_batch());
                        }
                    }
                }
                if let Some(id) = id {
                    let current: Option<String> = sqlx::query_scalar("SELECT group_deletion_operation_id FROM models_group WHERE group_id = ? AND group_user_id = ?")
                        .bind(id).bind(user.id).fetch_one(&mut *db).await?;
                    if current.as_deref() != Some(action.deletion_id()) {
                        return Err(invalid_batch());
                    }
                }
                if matches!(action, GroupLifecycleChange::Restore { .. }) {
                    let Some(id) = id else {
                        return Err(invalid_batch());
                    };
                    let within: bool = sqlx::query_scalar("SELECT COALESCE(group_deleted_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-' || ? || ' days'), 0) FROM models_group WHERE group_id = ? AND group_user_id = ?")
                        .bind(retention_days).bind(id).bind(user.id).fetch_one(&mut *db).await?;
                    if !within {
                        return Err(invalid_batch());
                    }
                }
            }
        }
    }
    Ok(())
}

async fn apply_lifecycle(
    db: &mut SqliteConnection,
    user: &User,
    groups: &mut HashMap<&str, GroupState>,
    lifecycle: &[&GroupLifecycleChange],
) -> Result<(), DbError> {
    for action in lifecycle {
        let (id, state) = groups
            .get_mut(action.group_id())
            .ok_or_else(invalid_batch)?;
        let now = time_now();
        match action {
            GroupLifecycleChange::Delete {
                deletion_operation_id,
                ..
            } => {
                let id = id.ok_or_else(invalid_batch)?;
                sqlx::query("DELETE FROM group_recovery_members WHERE group_id = ?")
                    .bind(id)
                    .execute(&mut *db)
                    .await?;
                sqlx::query("INSERT INTO group_recovery_members (group_id, model_id) SELECT ?, model_id FROM models WHERE model_group_id = ? AND model_user_id = ?")
                    .bind(id).bind(id).bind(user.id).execute(&mut *db).await?;
                sqlx::query("UPDATE models_group SET group_deleted_at = ?, group_deletion_operation_id = ? WHERE group_id = ? AND group_user_id = ?")
                    .bind(now).bind(deletion_operation_id).bind(id).bind(user.id).execute(&mut *db).await?;
                sqlx::query("UPDATE group_sync_heads SET last_deletion_operation_id = ? WHERE user_id = ? AND group_global_id = ?")
                    .bind(deletion_operation_id).bind(user.id).bind(action.group_id()).execute(&mut *db).await?;
                *state = "trashed".into();
            }
            GroupLifecycleChange::Restore { .. } | GroupLifecycleChange::Keep { .. } => {
                if let Some(existing) = *id {
                    match action {
                        GroupLifecycleChange::Keep { name, resource, .. } => {
                            let resource_id = resolve_keep_resource(db, user, resource).await?;
                            sqlx::query("UPDATE models_group SET group_deleted_at = NULL, group_last_modified = ?, group_name = ?, group_resource_id = ? WHERE group_id = ? AND group_user_id = ?").bind(&now).bind(name).bind(resource_id).bind(existing).bind(user.id).execute(&mut *db).await?;
                        }
                        _ => {
                            sqlx::query("UPDATE models_group SET group_deleted_at = NULL, group_last_modified = ? WHERE group_id = ? AND group_user_id = ?").bind(&now).bind(existing).bind(user.id).execute(&mut *db).await?;
                        }
                    }
                    sqlx::query("DELETE FROM group_recovery_members WHERE group_id = ?")
                        .bind(existing)
                        .execute(&mut *db)
                        .await?;
                } else if let GroupLifecycleChange::Keep {
                    group_global_id,
                    name,
                    expected_birth_id,
                    resource,
                    ..
                } = action
                {
                    let resource_id = resolve_keep_resource(db, user, resource).await?;
                    if let Some(birth) = expected_birth_id {
                        sqlx::query("INSERT INTO group_birth_rehydration_permits (user_id, birth_id, global_id, deletion_operation_id) VALUES (?, ?, ?, ?)")
                            .bind(user.id).bind(birth).bind(group_global_id).bind(action.deletion_id()).execute(&mut *db).await?;
                    }
                    let result = sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id, group_deletion_operation_id, group_birth_id) VALUES (?, ?, ?, ?, ?, ?, ?)")
                        .bind(name).bind(&now).bind(user.id).bind(&now).bind(group_global_id).bind(action.deletion_id()).bind(expected_birth_id).execute(&mut *db).await?;
                    let group_id = result.last_insert_rowid();
                    if let Some(resource_id) = resource_id {
                        sqlx::query("UPDATE models_group SET group_resource_id = ? WHERE group_id = ? AND group_user_id = ?")
                            .bind(resource_id).bind(group_id).bind(user.id).execute(&mut *db).await?;
                    }
                    *id = Some(group_id);
                } else {
                    return Err(invalid_batch());
                }
                sqlx::query("UPDATE group_deletion_records SET restored_at = ? WHERE user_id = ? AND group_global_id = ? AND operation_id = ? AND restored_at IS NULL")
                    .bind(now).bind(user.id).bind(action.group_id()).bind(action.deletion_id()).execute(&mut *db).await?;
                *state = "active".into();
            }
        }
    }
    Ok(())
}

async fn resolve_keep_resource(
    db: &mut SqliteConnection,
    user: &User,
    resource: &KeepResource,
) -> Result<Option<i64>, DbError> {
    match resource {
        KeepResource::None => Ok(None),
        KeepResource::Existing {
            resource_global_id,
            expected_incarnation,
        } => {
            if resource_global_id.is_empty() || expected_incarnation.is_empty() {
                return Err(invalid_batch());
            }
            let rows: Vec<(i64, String)> = sqlx::query_as("SELECT resource_id, resource_incarnation FROM resources WHERE resource_user_id = ? AND resource_unique_global_id = ?")
                .bind(user.id).bind(resource_global_id).fetch_all(db).await?;
            let [(id, incarnation)] = rows.as_slice() else {
                return Err(invalid_batch());
            };
            if incarnation != expected_incarnation {
                return Err(invalid_batch());
            }
            Ok(Some(*id))
        }
    }
}

fn invalid_batch() -> DbError {
    DbError::Protocol("Invalid or stale assignment batch".into())
}

fn validate_batch_shape(
    operation_id: &str,
    changes: &[&AssignmentChange],
    groups: &[&GroupPrecondition],
    lifecycle: &[&GroupLifecycleChange],
) -> Result<(), DbError> {
    if operation_id.is_empty()
        || changes.iter().any(|item| item.model_global_id.is_empty())
        || groups.iter().any(|item| item.group_global_id.is_empty())
        || lifecycle
            .iter()
            .any(|action| action.group_id().is_empty() || action.deletion_id().is_empty())
        || changes
            .windows(2)
            .any(|pair| pair[0].model_global_id == pair[1].model_global_id)
        || groups
            .windows(2)
            .any(|pair| pair[0].group_global_id == pair[1].group_global_id)
        || lifecycle
            .windows(2)
            .any(|pair| pair[0].group_id() == pair[1].group_id())
    {
        return Err(invalid_batch());
    }
    Ok(())
}

fn canonical_payload(
    changes: &[&AssignmentChange],
    groups: &[&GroupPrecondition],
    lifecycle: &[&GroupLifecycleChange],
    retention_days: u32,
) -> String {
    let mut payload = String::from("lifecycle-v1;");
    for change in changes {
        payload.push('m');
        encode(&mut payload, Some(&change.model_global_id));
        encode(&mut payload, Some(&change.expected_incarnation));
        encode(&mut payload, change.expected_group_global_id.as_deref());
        encode(&mut payload, Some(&change.expected_revision.to_string()));
        encode(
            &mut payload,
            Some(&change.expected_lifecycle_revision.to_string()),
        );
        encode(&mut payload, change.expected_deleted_at.as_deref());
        encode(&mut payload, change.target_group_global_id.as_deref());
    }
    for group in groups {
        payload.push('g');
        encode(&mut payload, Some(&group.group_global_id));
        encode(&mut payload, Some(&group.expected_generation.to_string()));
        encode(&mut payload, Some(&group.expected_head.to_string()));
        encode(&mut payload, Some(&group.expected_state));
        encode(
            &mut payload,
            group
                .expected_revision
                .map(|revision| revision.to_string())
                .as_deref(),
        );
        encode(&mut payload, group.expected_deleted_at.as_deref());
    }
    if !lifecycle.is_empty() {
        payload.push('l');
        encode(&mut payload, Some(&retention_days.to_string()));
        for action in lifecycle {
            match action {
                GroupLifecycleChange::Delete { .. } => payload.push('d'),
                GroupLifecycleChange::Restore { .. } => payload.push('r'),
                GroupLifecycleChange::Keep { .. } => payload.push('k'),
            }
            encode(&mut payload, Some(action.group_id()));
            encode(&mut payload, Some(action.deletion_id()));
            if let GroupLifecycleChange::Keep {
                name,
                expected_birth_id,
                resource,
                ..
            } = action
            {
                encode(&mut payload, Some(name));
                // Keep legacy retries stable when the request has no known birth.
                if let Some(birth_id) = expected_birth_id {
                    payload.push('b');
                    encode(&mut payload, Some(birth_id));
                }
                match resource {
                    KeepResource::None => payload.push('n'),
                    KeepResource::Existing {
                        resource_global_id,
                        expected_incarnation,
                    } => {
                        payload.push('p');
                        encode(&mut payload, Some(resource_global_id));
                        encode(&mut payload, Some(expected_incarnation));
                    }
                }
            }
        }
    }
    payload
}

fn encode(payload: &mut String, value: Option<&str>) {
    match value {
        Some(value) => {
            write!(payload, "{}:{value}", value.len()).expect("writing to a string cannot fail");
        }
        None => payload.push('-'),
    }
}
