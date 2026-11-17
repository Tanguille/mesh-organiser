use sqlx::SqliteConnection;

use super::{
    checked,
    graph::Graph,
    invalid,
    storage::{self, Scope},
};
use crate::{
    DbError,
    causal_envelope::{Action, EntityKind},
    db_context::DbContext,
    model::user::User,
};

pub(super) async fn absence_guard(
    db: &mut SqliteConnection,
    user: &User,
    scope: &Scope,
) -> Result<Option<String>, DbError> {
    sqlx::query_scalar("SELECT json_object('kind','purged_group','generation',h.generation,'head',h.head,'deletion_operation_id',r.deletion_operation_id) FROM group_lifecycle_group_rows r JOIN causal_genesis_bindings b ON b.user_id=r.user_id AND b.entity_kind='group' AND b.birth_id=r.birth_id JOIN entity_births e ON e.user_id=b.user_id AND e.entity_kind='group' AND e.birth_id=b.birth_id AND e.global_id=b.global_id JOIN group_sync_heads h ON h.user_id=b.user_id AND h.group_global_id=b.global_id JOIN group_deletion_records d ON d.user_id=b.user_id AND d.group_global_id=b.global_id AND d.group_birth_id=b.birth_id AND d.operation_id=r.deletion_operation_id WHERE r.user_id=? AND r.birth_id=? AND r.local_group_id=? AND r.state='purged' AND h.state='absent' AND h.identity_birth_id=b.birth_id AND h.last_deletion_operation_id=r.deletion_operation_id AND d.restored_at IS NULL AND r.retained_resource_id IS ? AND r.retained_resource_incarnation IS ? AND (r.retained_resource_id IS NULL OR EXISTS(SELECT 1 FROM resources p WHERE p.resource_id=r.retained_resource_id AND p.resource_user_id=r.user_id AND p.resource_incarnation=r.retained_resource_incarnation)) AND NOT EXISTS(SELECT 1 FROM models_group g WHERE g.group_user_id=b.user_id AND (g.group_birth_id=b.birth_id OR g.group_unique_global_id=b.global_id))")
        .bind(user.id).bind(&scope.entity.birth_id).bind(scope.local_id).bind(scope.project_id).bind(&scope.project_incarnation).fetch_optional(db).await
}

pub(super) async fn purge(db: &DbContext, retention_days: u32) -> Result<u64, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let owners:Vec<i64>=sqlx::query_scalar("SELECT DISTINCT g.group_user_id FROM models_group g JOIN group_lifecycle_group_rows r ON r.user_id=g.group_user_id AND r.local_group_id=g.group_id AND r.birth_id=g.group_birth_id WHERE g.group_deleted_at < strftime('%Y-%m-%dT%H:%M:%SZ','now','-' || ? || ' days')")
        .bind(retention_days).fetch_all(&mut *tx).await?;
    for owner in owners {
        let user = User {
            id: owner,
            ..User::default()
        };
        let (mut scopes, graph) = checked(&mut tx, &user).await?;
        if scopes.iter().any(|scope| scope.paused) {
            continue;
        }
        for scope in scopes
            .iter_mut()
            .filter(|scope| scope.entity.kind == EntityKind::Group && !scope.purged)
        {
            if graph.active(&scope.entity, None) {
                continue;
            }
            certify(&mut tx, &user, scope, &graph, retention_days).await?;
        }
    }
    let deleted=sqlx::query("DELETE FROM models_group WHERE group_deleted_at < strftime('%Y-%m-%dT%H:%M:%SZ','now','-' || ? || ' days')")
        .bind(retention_days).execute(&mut *tx).await?.rows_affected();
    // Retirement advances the local head. Record that exact absent incarnation;
    // future same-global reuse or an unexplained head change invalidates it.
    let absent:Vec<(i64,String)>=sqlx::query_as("SELECT r.user_id,r.birth_id FROM group_lifecycle_group_rows r JOIN group_lifecycle_scopes s ON s.user_id=r.user_id AND s.entity_kind='group' AND s.birth_id=r.birth_id WHERE r.state='purged' AND s.paused=0")
        .fetch_all(&mut *tx).await?;
    for (owner, birth) in absent {
        sqlx::query("UPDATE group_lifecycle_scopes SET guard=(SELECT json_object('kind','purged_group','generation',h.generation,'head',h.head,'deletion_operation_id',r.deletion_operation_id) FROM group_lifecycle_group_rows r JOIN causal_genesis_bindings b ON b.user_id=r.user_id AND b.entity_kind='group' AND b.birth_id=r.birth_id JOIN group_sync_heads h ON h.user_id=b.user_id AND h.group_global_id=b.global_id WHERE r.user_id=? AND r.birth_id=? AND h.state='absent'),revision=revision+1 WHERE user_id=? AND entity_kind='group' AND birth_id=? AND json_extract(guard,'$.kind')='group'")
            .bind(owner).bind(&birth).bind(owner).bind(&birth).execute(&mut *tx).await?;
    }
    tx.commit().await?;

    Ok(deleted)
}

async fn certify(
    db: &mut SqliteConnection,
    user: &User,
    scope: &Scope,
    graph: &Graph,
    days: u32,
) -> Result<(), DbError> {
    let row:Option<(String,String,String,String)>=sqlx::query_as("SELECT group_deletion_operation_id,group_name,group_created,group_last_modified FROM models_group WHERE group_id=? AND group_user_id=? AND group_birth_id=? AND group_unique_global_id=? AND group_resource_id IS ? AND group_deleted_at < strftime('%Y-%m-%dT%H:%M:%SZ','now','-' || ? || ' days') AND NOT EXISTS(SELECT 1 FROM models WHERE model_group_id=models_group.group_id) AND length(CAST(group_name AS BLOB))+length(CAST(group_created AS BLOB))+length(CAST(group_last_modified AS BLOB))<=2097152")
        .bind(scope.local_id).bind(user.id).bind(&scope.entity.birth_id).bind(&scope.entity.global_id).bind(scope.project_id).bind(days).fetch_optional(&mut *db).await?;
    let Some((operation, name, created, modified)) = row else {
        return Ok(());
    };
    let frontier = graph.frontier(&scope.entity, None);
    if !graph.events.iter().any(|event|event.entity==scope.entity&&frontier.contains(&event.event)&&matches!(&event.action,Action::GroupDeleted{deletion_operation_id} if deletion_operation_id==&operation)){return Ok(());}
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_deletion_records WHERE user_id=? AND group_global_id=? AND group_birth_id=? AND operation_id=? AND restored_at IS NULL)")
        .bind(user.id).bind(&scope.entity.global_id).bind(&scope.entity.birth_id).bind(&operation).fetch_one(&mut *db).await?;
    if !valid {
        return Ok(());
    }
    let retained_bytes:i64=sqlx::query_scalar("SELECT COALESCE(SUM(COALESCE(length(CAST(retained_name AS BLOB)),0)+COALESCE(length(CAST(retained_created AS BLOB)),0)+COALESCE(length(CAST(retained_last_modified AS BLOB)),0)),0) FROM group_lifecycle_group_rows WHERE user_id=? AND birth_id!=?")
        .bind(user.id).bind(&scope.entity.birth_id).fetch_one(&mut *db).await?;
    if usize::try_from(retained_bytes)
        .map_err(|_| invalid())?
        .saturating_add(name.len())
        .saturating_add(created.len())
        .saturating_add(modified.len())
        > crate::causal_store::MAX_OWNER_BYTES
    {
        return Ok(());
    }
    sqlx::query("UPDATE group_sync_heads SET last_deletion_operation_id=? WHERE user_id=? AND group_global_id=? AND identity_birth_id=? AND state='trashed'")
        .bind(&operation).bind(user.id).bind(&scope.entity.global_id).bind(&scope.entity.birth_id).execute(&mut *db).await?;
    sqlx::query("UPDATE group_lifecycle_group_rows SET state='purged',deletion_operation_id=?,retained_name=?,retained_created=?,retained_last_modified=?,retained_resource_id=?,retained_resource_incarnation=? WHERE user_id=? AND birth_id=? AND local_group_id=? AND state='present'")
        .bind(operation).bind(name).bind(created).bind(modified).bind(scope.project_id).bind(&scope.project_incarnation).bind(user.id).bind(&scope.entity.birth_id).bind(scope.local_id).execute(db).await?;

    Ok(())
}

pub(super) async fn materialize(
    db: &mut SqliteConnection,
    user: &User,
    scopes: &mut [Scope],
    graph: &Graph,
) -> Result<(), DbError> {
    for scope in scopes
        .iter_mut()
        .filter(|scope| scope.purged && graph.active(&scope.entity, None))
    {
        let guard = absence_guard(db, user, scope).await?.ok_or_else(invalid)?;
        let expected:String=sqlx::query_scalar("SELECT guard FROM group_lifecycle_scopes WHERE user_id=? AND entity_kind='group' AND birth_id=? AND paused=0")
            .bind(user.id).bind(&scope.entity.birth_id).fetch_one(&mut *db).await?;
        if guard != expected {
            return Err(invalid());
        }
        let (operation,cached_name,created,modified):(String,String,String,String)=sqlx::query_as("SELECT deletion_operation_id,retained_name,retained_created,retained_last_modified FROM group_lifecycle_group_rows WHERE user_id=? AND birth_id=? AND state='purged'")
            .bind(user.id).bind(&scope.entity.birth_id).fetch_one(&mut *db).await?;
        let names = graph.names(&scope.entity, None);
        let name = if names.len() == 1 {
            &names[0].name
        } else {
            &cached_name
        };
        sqlx::query("INSERT INTO group_birth_rehydration_permits(user_id,birth_id,global_id,deletion_operation_id) VALUES(?,?,?,?)")
            .bind(user.id).bind(&scope.entity.birth_id).bind(&scope.entity.global_id).bind(operation).execute(&mut *db).await?;
        let id=sqlx::query("INSERT INTO models_group(group_name,group_created,group_last_modified,group_user_id,group_unique_global_id,group_birth_id,group_resource_id) VALUES(?,?,?,?,?,?,?)")
            .bind(name).bind(created).bind(modified).bind(user.id).bind(&scope.entity.global_id).bind(&scope.entity.birth_id).bind(scope.project_id).execute(&mut *db).await?.last_insert_rowid();
        sqlx::query("UPDATE group_lifecycle_group_rows SET local_group_id=?,state='present',deletion_operation_id=NULL,retained_name=NULL,retained_created=NULL,retained_last_modified=NULL,retained_resource_id=NULL,retained_resource_incarnation=NULL WHERE user_id=? AND birth_id=? AND state='purged'")
            .bind(id).bind(user.id).bind(&scope.entity.birth_id).execute(&mut *db).await?;
        scope.local_id = id;
        scope.purged = false;
        let current = storage::current_guard(db, user, scope)
            .await?
            .ok_or_else(invalid)?;
        scope.generation = serde_json::from_str::<serde_json::Value>(&current)
            .map_err(|_| invalid())?["generation"]
            .as_i64()
            .ok_or_else(invalid)?;
    }

    Ok(())
}
