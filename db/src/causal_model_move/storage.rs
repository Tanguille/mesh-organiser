use sqlx::SqliteConnection;

use super::{counter, invalid};
use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, Dot, Entity, EntityKind},
    causal_store::{self, IdentityStatus},
    model::user::User,
};

pub(super) struct Scope {
    pub user_id: i64,
    pub birth_id: String,
    pub local_id: i64,
    pub incarnation: String,
    pub revision: i64,
    pub root: CausalEnvelope,
    pub root_wire: String,
    pub paused: bool,
    pub current_group_id: Option<i64>,
    pub current_destination: Option<Entity>,
}

#[derive(sqlx::FromRow)]
struct Row {
    birth_id: String,
    global_id: String,
    local_id: i64,
    incarnation: String,
    guard_assignment_revision: i64,
    assignment_revision: i64,
    revision: i64,
    paused: bool,
    root_wire: String,
    group_id: Option<i64>,
    group_global: Option<String>,
    group_birth: Option<String>,
}

pub(super) async fn load(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
) -> Result<Option<Scope>, DbError> {
    let row: Option<Row> = sqlx::query_as("SELECT s.birth_id, b.global_id, b.local_id, m.model_incarnation AS incarnation, s.guard_assignment_revision, m.model_assignment_revision AS assignment_revision, s.revision, s.paused, e.envelope AS root_wire, m.model_group_id AS group_id, g.group_unique_global_id AS group_global, g.group_birth_id AS group_birth FROM model_move_scopes s JOIN causal_genesis_bindings b ON b.user_id = s.user_id AND b.entity_kind = 'model' AND b.birth_id = s.birth_id JOIN models m ON m.model_id = b.local_id AND m.model_user_id = b.user_id AND m.model_birth_id = b.birth_id AND m.model_unique_global_id = b.global_id JOIN blobs file ON file.blob_id = m.model_blob_id LEFT JOIN models_group g ON g.group_id = m.model_group_id AND g.group_user_id = m.model_user_id JOIN causal_events e ON e.user_id = b.user_id AND e.writer_id = b.writer_id AND e.counter = b.counter JOIN sync_accounts a ON a.user_id = s.user_id JOIN sync_instance i ON i.singleton = 1 WHERE s.user_id = ? AND b.local_id = ? AND s.invalidated = 0 AND m.model_deleted IS NULL AND m.model_incarnation = json_extract(b.local_guard, '$.incarnation') AND m.model_blob_id = json_extract(b.local_guard, '$.blob_id') AND file.blob_sha256 = json_extract(b.materialization, '$.blob.hash') AND file.blob_filetype = json_extract(b.materialization, '$.blob.filetype') AND file.blob_size = json_extract(b.materialization, '$.blob.size') AND m.model_lifecycle_revision = s.guard_lifecycle_revision AND b.local_instance_id = i.instance_id AND b.local_account_id = a.account_id AND e.encoding = 'portable_v2'")
        .bind(user.id).bind(id).fetch_optional(&mut *db).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if !row.paused && row.assignment_revision != row.guard_assignment_revision {
        return Ok(None);
    }
    let current_destination = match (row.group_id, row.group_global) {
        (None, None) => None,
        (Some(_), Some(global_id)) => Some(Entity {
            kind: EntityKind::Group,
            global_id,
            birth_id: row.group_birth,
        }),
        _ => return Ok(None),
    };
    let root = causal_envelope::parse_causal_envelope(&row.root_wire).map_err(|_| invalid())?;
    if root.entity.global_id != row.global_id
        || root.entity.birth_id.as_deref() != Some(&row.birth_id)
        || !matches!(root.action, Action::ModelCreated)
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
        incarnation: row.incarnation,
        revision: row.revision,
        root,
        root_wire: row.root_wire,
        paused: row.paused,
        current_group_id: row.group_id,
        current_destination,
    }))
}

pub(super) async fn pause(db: &mut SqliteConnection, scope: &mut Scope) -> Result<(), DbError> {
    let revision = scope.revision.checked_add(1).ok_or_else(invalid)?;
    sqlx::query(
        "UPDATE model_move_scopes SET paused = 1, revision = ? WHERE user_id = ? AND birth_id = ?",
    )
    .bind(revision)
    .bind(scope.user_id)
    .bind(&scope.birth_id)
    .execute(db)
    .await?;
    scope.revision = revision;
    scope.paused = true;

    Ok(())
}

pub(super) async fn invalidate(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
) -> Result<(), DbError> {
    sqlx::query("UPDATE model_move_scopes SET invalidated = 1 WHERE user_id = ? AND birth_id IN (SELECT birth_id FROM causal_genesis_bindings WHERE user_id = ? AND entity_kind = 'model' AND local_id = ?)")
        .bind(user.id).bind(user.id).bind(id).execute(db).await?;

    Ok(())
}

pub(super) async fn local_id(
    db: &mut SqliteConnection,
    user: &User,
    birth: &str,
) -> Result<i64, DbError> {
    sqlx::query_scalar("SELECT local_id FROM causal_genesis_bindings WHERE user_id = ? AND entity_kind = 'model' AND birth_id = ?")
        .bind(user.id).bind(birth).fetch_optional(db).await?.ok_or_else(invalid)
}

pub(super) async fn has_event(
    db: &mut SqliteConnection,
    scope: &Scope,
    dot: &Dot,
) -> Result<bool, DbError> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM model_move_events WHERE user_id = ? AND birth_id = ? AND writer_id = ? AND counter = ?)")
        .bind(scope.user_id).bind(&scope.birth_id).bind(&dot.writer_id).bind(counter(dot)?).fetch_one(db).await
}

pub(super) async fn metadata(
    db: &mut SqliteConnection,
    scope: &Scope,
    dot: &Dot,
    origin: &str,
) -> Result<(), DbError> {
    sqlx::query("INSERT INTO model_move_events(user_id, birth_id, writer_id, counter, provenance) VALUES (?, ?, ?, ?, ?)")
        .bind(scope.user_id).bind(&scope.birth_id).bind(&dot.writer_id).bind(counter(dot)?).bind(origin).execute(db).await?;

    Ok(())
}
