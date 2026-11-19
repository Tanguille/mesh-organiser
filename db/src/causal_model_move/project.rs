use super::invalid;

pub(super) async fn apply(
    db: &mut sqlx::SqliteConnection,
    user: &crate::model::user::User,
    scope: &mut super::storage::Scope,
    graph: &super::graph::Graph,
    timestamp: &str,
) -> Result<(), crate::DbError> {
    let revision = scope.revision.checked_add(1).ok_or_else(invalid)?;
    if graph.missing.is_empty() && graph.choices.len() == 1 {
        let destination = &graph.choices[0].destination;
        let target_id = if let Some(entity) = destination {
            Some(
                super::targets::by_entity(db, user, entity)
                    .await?
                    .ok_or_else(invalid)?
                    .id,
            )
        } else {
            None
        };
        sqlx::query("UPDATE models SET model_group_id = ? WHERE model_id = ? AND model_user_id = ? AND model_birth_id = ? AND model_incarnation = ? AND model_deleted IS NULL")
            .bind(target_id).bind(scope.local_id).bind(scope.user_id).bind(&scope.birth_id).bind(&scope.incarnation).execute(&mut *db).await?;
        if target_id != scope.current_group_id {
            sqlx::query("UPDATE models_group SET group_last_modified = ? WHERE group_user_id = ? AND group_id IN (?, ?)")
                .bind(timestamp).bind(scope.user_id).bind(scope.current_group_id).bind(target_id).execute(&mut *db).await?;
        }
        scope.current_group_id = target_id;
        scope.current_destination.clone_from(destination);
    }
    // Conflicts retain the local materialization. The frontier, never that cached
    // assignment, determines whether a later action needs an explicit choice.
    let (assignment, lifecycle): (i64, i64) = sqlx::query_as("SELECT model_assignment_revision, model_lifecycle_revision FROM models WHERE model_id = ? AND model_user_id = ? AND model_incarnation = ?")
        .bind(scope.local_id).bind(scope.user_id).bind(&scope.incarnation).fetch_one(&mut *db).await?;
    let updated = sqlx::query("UPDATE model_move_scopes SET guard_assignment_revision = ?, guard_lifecycle_revision = ?, revision = ? WHERE user_id = ? AND birth_id = ? AND invalidated = 0 AND paused = 0")
        .bind(assignment).bind(lifecycle).bind(revision).bind(scope.user_id).bind(&scope.birth_id).execute(&mut *db).await?;
    if updated.rows_affected() != 1 {
        return Err(invalid());
    }
    sqlx::query("DELETE FROM model_move_frontier WHERE user_id = ? AND birth_id = ?")
        .bind(scope.user_id)
        .bind(&scope.birth_id)
        .execute(&mut *db)
        .await?;
    for (dot, destination) in &graph.frontier {
        let target = destination
            .as_ref()
            .and_then(|entity| entity.birth_id.as_deref());
        sqlx::query("INSERT INTO model_move_frontier(user_id, birth_id, writer_id, counter, target_birth_id) VALUES (?, ?, ?, ?, ?)")
            .bind(scope.user_id).bind(&scope.birth_id).bind(&dot.writer_id).bind(super::counter(dot)?).bind(target).execute(&mut *db).await?;
    }
    scope.revision = revision;

    Ok(())
}
