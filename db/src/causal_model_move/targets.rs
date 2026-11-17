use sqlx::SqliteConnection;

use super::invalid;
use crate::{
    DbError,
    causal_envelope::{self, Action, Dot, Entity, EntityKind},
    causal_store::{self, IdentityStatus},
    model::user::User,
};

pub(super) struct Target {
    pub id: i64,
    pub entity: Entity,
    pub root: Dot,
}

pub(super) async fn by_entity(
    db: &mut SqliteConnection,
    user: &User,
    entity: &Entity,
) -> Result<Option<Target>, DbError> {
    if entity.kind != EntityKind::Group || entity.birth_id.is_none() {
        return Err(invalid());
    }
    let id: Option<i64> = sqlx::query_scalar("SELECT local_id FROM causal_genesis_bindings WHERE user_id = ? AND entity_kind = 'group' AND birth_id = ? AND global_id = ?")
        .bind(user.id).bind(&entity.birth_id).bind(&entity.global_id).fetch_optional(&mut *db).await?;
    let Some(id) = id else {
        return Ok(None);
    };

    by_id(db, user, id).await
}

pub(super) async fn by_id(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
) -> Result<Option<Target>, DbError> {
    // Names and memberships may change; only the checked initial lifecycle is
    // supported here. Deletion, restoration, rekey and project changes retire it.
    let row: Option<(String, String, String)> = sqlx::query_as("SELECT g.group_unique_global_id, g.group_birth_id, e.envelope FROM group_move_roots r JOIN causal_genesis_bindings b ON b.user_id = r.user_id AND b.entity_kind = 'group' AND b.birth_id = r.birth_id JOIN models_group g ON g.group_id = b.local_id AND g.group_user_id = b.user_id AND g.group_birth_id = b.birth_id AND g.group_unique_global_id = b.global_id JOIN group_sync_heads h ON h.user_id = g.group_user_id AND h.group_global_id = g.group_unique_global_id JOIN causal_events e ON e.user_id = b.user_id AND e.writer_id = b.writer_id AND e.counter = b.counter JOIN sync_accounts a ON a.user_id = r.user_id JOIN sync_instance i ON i.singleton = 1 WHERE r.user_id = ? AND b.local_id = ? AND r.invalidated = 0 AND g.group_deleted_at IS NULL AND g.group_resource_id IS NULL AND h.state = 'active' AND h.generation = json_extract(b.local_guard, '$.generation') AND b.local_instance_id = i.instance_id AND b.local_account_id = a.account_id AND e.encoding = 'portable_v2'")
        .bind(user.id).bind(id).fetch_optional(&mut *db).await?;
    let Some((global_id, birth_id, wire)) = row else {
        return Ok(None);
    };
    let entity = Entity {
        kind: EntityKind::Group,
        global_id,
        birth_id: Some(birth_id),
    };
    let root = causal_envelope::parse_causal_envelope(&wire).map_err(|_| invalid())?;
    if root.entity != entity
        || !matches!(root.action, Action::GroupCreated { .. })
        || !root.parents.is_empty()
        || !root.dependencies.is_empty()
        || causal_store::identity_status(db, user.id, &entity).await? != IdentityStatus::Uncontested
    {
        return Ok(None);
    }

    Ok(Some(Target {
        id,
        entity,
        root: root.event,
    }))
}
