use sqlx::SqliteConnection;

use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, Dot},
    causal_store::{self, IdentityStatus},
    model::user::User,
};

use super::{graph::Graph, invalid};

pub(super) struct Scope {
    pub user_id: i64,
    pub birth_id: String,
    pub local_id: i64,
    pub generation: i64,
    pub revision: i64,
    pub root: CausalEnvelope,
    pub root_wire: String,
    pub paused: bool,
    pub current_name: String,
}

#[derive(sqlx::FromRow)]
struct ScopeRow {
    birth_id: String,
    global_id: String,
    local_id: i64,
    generation: i64,
    guard_head: i64,
    guard_revision: i64,
    revision: i64,
    root_wire: String,
    paused: bool,
}

pub(super) async fn load(
    db: &mut SqliteConnection,
    user: &User,
    group_id: i64,
) -> Result<Option<Scope>, DbError> {
    // Pins are checked here too because ordinary edit_group has no caller-supplied
    // sync identity; a replaced account cannot inherit an old scope by numeric id.
    let row: Option<ScopeRow> = sqlx::query_as("SELECT s.birth_id, b.global_id, b.local_id, json_extract(b.local_guard, '$.generation') AS generation, s.guard_head, s.guard_revision, s.revision, s.paused, e.envelope AS root_wire FROM group_rename_scopes s JOIN causal_genesis_bindings b ON b.user_id = s.user_id AND b.entity_kind = 'group' AND b.birth_id = s.birth_id JOIN causal_events e ON e.user_id = b.user_id AND e.writer_id = b.writer_id AND e.counter = b.counter JOIN sync_accounts a ON a.user_id = s.user_id JOIN sync_instance i ON i.singleton = 1 WHERE s.user_id = ? AND b.local_id = ? AND s.invalidated = 0 AND b.local_instance_id = i.instance_id AND b.local_account_id = a.account_id AND e.encoding = 'portable_v2'")
        .bind(user.id).bind(group_id).fetch_optional(&mut *db).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let current: Option<(i64, i64, i64, String)> = sqlx::query_as("SELECT h.generation, h.head, g.group_revision, g.group_name FROM models_group g JOIN group_sync_heads h ON h.user_id = g.group_user_id AND h.group_global_id = g.group_unique_global_id WHERE g.group_id = ? AND g.group_user_id = ? AND g.group_birth_id = ? AND g.group_unique_global_id = ? AND g.group_deleted_at IS NULL AND g.group_resource_id IS NULL AND h.state = 'active' AND NOT EXISTS(SELECT 1 FROM models WHERE model_group_id = g.group_id)")
        .bind(row.local_id).bind(user.id).bind(&row.birth_id).bind(&row.global_id).fetch_optional(&mut *db).await?;
    let Some((generation, head, revision, current_name)) = current else {
        return Ok(None);
    };
    if generation != row.generation
        || (!row.paused && (head != row.guard_head || revision != row.guard_revision))
    {
        return Ok(None);
    }
    let root = causal_envelope::parse_causal_envelope(&row.root_wire).map_err(|_| invalid())?;
    if root.entity.global_id != row.global_id
        || root.entity.birth_id.as_deref() != Some(&row.birth_id)
        || !matches!(root.action, Action::GroupCreated { .. })
        || !root.parents.is_empty()
        || !root.dependencies.is_empty()
        || causal_store::identity_status(db, user.id, &root.entity).await?
            != IdentityStatus::Uncontested
    {
        return Ok(None);
    }

    Ok(Some(Scope {
        user_id: user.id,
        birth_id: row.birth_id,
        local_id: row.local_id,
        generation: row.generation,
        revision: row.revision,
        root,
        root_wire: row.root_wire,
        paused: row.paused,
        current_name,
    }))
}

pub(super) async fn pause(db: &mut SqliteConnection, scope: &mut Scope) -> Result<(), DbError> {
    let revision = scope.revision.checked_add(1).ok_or_else(invalid)?;
    sqlx::query("UPDATE group_rename_scopes SET paused = 1, revision = ? WHERE user_id = ? AND birth_id = ?")
        .bind(revision).bind(scope.user_id).bind(&scope.birth_id).execute(db).await?;
    scope.revision = revision;
    scope.paused = true;

    Ok(())
}

pub(super) async fn invalidate(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
) -> Result<(), DbError> {
    sqlx::query("UPDATE group_rename_scopes SET invalidated = 1 WHERE user_id = ? AND birth_id IN (SELECT birth_id FROM causal_genesis_bindings WHERE user_id = ? AND entity_kind = 'group' AND local_id = ?)")
        .bind(user.id).bind(user.id).bind(id).execute(db).await?;

    Ok(())
}

pub(super) async fn local_id(
    db: &mut SqliteConnection,
    user: &User,
    birth: &str,
) -> Result<i64, DbError> {
    sqlx::query_scalar("SELECT local_id FROM causal_genesis_bindings WHERE user_id = ? AND entity_kind = 'group' AND birth_id = ?")
        .bind(user.id).bind(birth).fetch_optional(db).await?.ok_or_else(invalid)
}

pub(super) async fn has_event(
    db: &mut SqliteConnection,
    scope: &Scope,
    dot: &Dot,
) -> Result<bool, DbError> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_rename_events WHERE user_id = ? AND birth_id = ? AND writer_id = ? AND counter = ?)")
        .bind(scope.user_id).bind(&scope.birth_id).bind(&dot.writer_id).bind(super::counter(dot)?).fetch_one(db).await
}

pub(super) async fn metadata(
    db: &mut SqliteConnection,
    scope: &Scope,
    dot: &Dot,
    origin: &str,
) -> Result<(), DbError> {
    sqlx::query("INSERT INTO group_rename_events(user_id, birth_id, writer_id, counter, provenance) VALUES (?, ?, ?, ?, ?)")
        .bind(scope.user_id).bind(&scope.birth_id).bind(&dot.writer_id).bind(super::counter(dot)?).bind(origin).execute(db).await?;

    Ok(())
}

pub(super) async fn project(
    db: &mut SqliteConnection,
    scope: &mut Scope,
    graph: &Graph,
    timestamp: &str,
) -> Result<(), DbError> {
    let revision = scope.revision.checked_add(1).ok_or_else(invalid)?;
    if let Some(name) = graph.projection() {
        sqlx::query("UPDATE models_group SET group_name = ?, group_last_modified = ? WHERE group_id = ? AND group_user_id = ? AND group_birth_id = ?")
            .bind(name).bind(timestamp).bind(scope.local_id).bind(scope.user_id).bind(&scope.birth_id).execute(&mut *db).await?;
    }
    let (head, group_revision): (i64, i64) = sqlx::query_as("SELECT h.head, g.group_revision FROM models_group g JOIN group_sync_heads h ON h.user_id = g.group_user_id AND h.group_global_id = g.group_unique_global_id WHERE g.group_id = ? AND g.group_user_id = ?")
        .bind(scope.local_id).bind(scope.user_id).fetch_one(&mut *db).await?;
    sqlx::query("UPDATE group_rename_scopes SET guard_head = ?, guard_revision = ?, revision = ? WHERE user_id = ? AND birth_id = ? AND invalidated = 0")
        .bind(head).bind(group_revision).bind(revision).bind(scope.user_id).bind(&scope.birth_id).execute(&mut *db).await?;
    sqlx::query("DELETE FROM group_rename_frontier WHERE user_id = ? AND birth_id = ?")
        .bind(scope.user_id)
        .bind(&scope.birth_id)
        .execute(&mut *db)
        .await?;
    for dot in &graph.frontier {
        sqlx::query("INSERT INTO group_rename_frontier(user_id, birth_id, writer_id, counter) VALUES (?, ?, ?, ?)")
            .bind(scope.user_id).bind(&scope.birth_id).bind(&dot.writer_id).bind(super::counter(dot)?).execute(&mut *db).await?;
    }
    scope.revision = revision;

    Ok(())
}
