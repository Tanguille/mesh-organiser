use sqlx::SqliteConnection;

use crate::{
    DbError,
    causal_envelope::{Action, EntityKind},
    model::user::User,
};

use super::{
    graph::Graph,
    invalid,
    storage::{self, Scope},
};

pub(super) async fn apply(
    db: &mut SqliteConnection,
    user: &User,
    scopes: &mut [Scope],
    graph: &Graph,
) -> Result<(), DbError> {
    super::retention::materialize(db, user, scopes, graph).await?;
    let timestamp = crate::time_now();
    history(db, user, scopes, graph, &timestamp).await?;
    for scope in scopes
        .iter()
        .filter(|scope| scope.entity.kind == EntityKind::Group && !scope.purged)
    {
        let names = graph.names(&scope.entity, None);
        if names.len() == 1 {
            sqlx::query("UPDATE models_group SET group_name=? WHERE group_id=? AND group_user_id=? AND group_birth_id=?")
                .bind(&names[0].name).bind(scope.local_id).bind(user.id).bind(&scope.entity.birth_id).execute(&mut *db).await?;
        }
        if graph.active(&scope.entity, None) {
            sqlx::query("UPDATE models_group SET group_deleted_at=NULL,group_deletion_operation_id=NULL WHERE group_id=? AND group_user_id=? AND group_birth_id=?")
                .bind(scope.local_id).bind(user.id).bind(&scope.entity.birth_id).execute(&mut *db).await?;
        } else {
            let frontier = graph.frontier(&scope.entity, None);
            let deletion = graph
                .events
                .iter()
                .find(|event| event.event == frontier[0])
                .ok_or_else(invalid)?;
            let Action::GroupDeleted {
                deletion_operation_id,
            } = &deletion.action
            else {
                return Err(invalid());
            };
            sqlx::query("UPDATE models_group SET group_deleted_at=COALESCE(group_deleted_at,?),group_deletion_operation_id=? WHERE group_id=? AND group_user_id=? AND group_birth_id=?")
                .bind(&timestamp).bind(deletion_operation_id).bind(scope.local_id).bind(user.id).bind(&scope.entity.birth_id).execute(&mut *db).await?;
        }
    }
    for scope in scopes
        .iter()
        .filter(|scope| scope.entity.kind == EntityKind::Model)
    {
        let choices = graph.moves(&scope.entity, None);
        if choices.len() == 1 {
            let destination = choices[0]
                .destination
                .as_ref()
                .map(|entity| {
                    scopes
                        .iter()
                        .find(|target| target.entity == *entity)
                        .map(|target| target.local_id)
                        .ok_or_else(invalid)
                })
                .transpose()?;
            sqlx::query("UPDATE models SET model_group_id=? WHERE model_id=? AND model_user_id=? AND model_incarnation=?")
                .bind(destination).bind(scope.local_id).bind(user.id).bind(&scope.incarnation).execute(&mut *db).await?;
        } else {
            // A cached destination is not a conflict choice. Deleting that
            // visible group must still ungroup its row without resolving H/J.
            sqlx::query("UPDATE models SET model_group_id=NULL WHERE model_id=? AND model_user_id=? AND model_incarnation=? AND EXISTS(SELECT 1 FROM models_group g WHERE g.group_id=models.model_group_id AND g.group_user_id=models.model_user_id AND g.group_deleted_at IS NOT NULL)")
                .bind(scope.local_id).bind(user.id).bind(&scope.incarnation).execute(&mut *db).await?;
        }
    }
    // All guards were checked before projection; only this complete operation may
    // refresh them. Unsupported domain changes are never silently adopted.
    for scope in scopes {
        let guard = storage::current_guard(db, user, scope)
            .await?
            .ok_or_else(invalid)?;
        scope.revision = scope.revision.checked_add(1).ok_or_else(invalid)?;
        sqlx::query("UPDATE group_lifecycle_scopes SET guard=?,revision=? WHERE user_id=? AND entity_kind=? AND birth_id=? AND paused=0")
            .bind(guard).bind(scope.revision).bind(user.id).bind(if scope.entity.kind==EntityKind::Group{"group"}else{"model"}).bind(&scope.entity.birth_id).execute(&mut *db).await?;
    }

    Ok(())
}

async fn history(
    db: &mut SqliteConnection,
    user: &User,
    scopes: &[Scope],
    graph: &Graph,
    timestamp: &str,
) -> Result<(), DbError> {
    // Keep deletion facts even when a concurrent keep-alive makes the group active.
    for event in &graph.events {
        if let Action::GroupDeleted {
            deletion_operation_id,
        } = &event.action
        {
            sqlx::query("INSERT OR IGNORE INTO group_deletion_records(user_id,group_global_id,operation_id,deleted_at,group_revision,group_birth_id) VALUES(?,?,?,?,0,?)")
                .bind(user.id).bind(&event.entity.global_id).bind(deletion_operation_id).bind(timestamp).bind(&event.entity.birth_id).execute(&mut *db).await?;
        }
    }
    for event in &graph.events {
        if let Action::GroupRestored { deletion } = &event.action {
            let original = graph
                .events
                .iter()
                .find(|candidate| candidate.event == *deletion)
                .ok_or_else(invalid)?;
            let Action::GroupDeleted {
                deletion_operation_id,
            } = &original.action
            else {
                return Err(invalid());
            };
            sqlx::query("UPDATE group_deletion_records SET restored_at=COALESCE(restored_at,?) WHERE user_id=? AND operation_id=?")
                .bind(timestamp).bind(user.id).bind(deletion_operation_id).execute(&mut *db).await?;
        }
        if let Action::ModelDetached { group, .. } = &event.action {
            let group = scopes
                .iter()
                .find(|scope| scope.entity == *group)
                .ok_or_else(invalid)?;
            if group.purged {
                continue;
            }
            let model = scopes
                .iter()
                .find(|scope| scope.entity == event.entity)
                .ok_or_else(invalid)?;
            sqlx::query(
                "INSERT OR IGNORE INTO group_recovery_members(group_id,model_id) VALUES(?,?)",
            )
            .bind(group.local_id)
            .bind(model.local_id)
            .execute(&mut *db)
            .await?;
        }
    }

    Ok(())
}
