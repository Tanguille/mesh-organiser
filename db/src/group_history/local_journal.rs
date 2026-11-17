use sqlx::SqliteConnection;

use crate::{DbError, model::user::User};

use super::EventId;

#[derive(Default, serde::Serialize)]
pub(super) struct GroupFact<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub deletion_operation_id: Option<&'a str>,
    pub deletion_replica_id: Option<&'a str>,
    pub deletion_counter: Option<i64>,
    pub model_global_id: Option<&'a str>,
    pub model_incarnation: Option<&'a str>,
    pub membership_replica_id: Option<&'a str>,
    pub membership_counter: Option<i64>,
}

#[derive(serde::Serialize)]
pub(super) struct MembershipFact<'a> {
    pub kind: &'a str,
    pub group_global_id: &'a str,
    pub deletion_operation_id: &'a str,
    pub deletion_replica_id: Option<&'a str>,
    pub deletion_counter: Option<i64>,
    pub from_group_global_id: Option<&'a str>,
    pub to_group_global_id: Option<&'a str>,
}

#[derive(serde::Serialize)]
struct LocalEntity<'a> {
    kind: &'a str,
    global_id: &'a str,
    birth_id: Option<&'a str>,
    local_incarnation: Option<&'a str>,
}

#[derive(serde::Serialize)]
struct LocalHistory<'a, T> {
    entity: LocalEntity<'a>,
    ancestry: &'a str,
    action: &'a T,
}

async fn next_event(db: &mut SqliteConnection, user: &User) -> Result<EventId, DbError> {
    let (replica_id, counter) = crate::causal_clock::next(db, user.id).await?;

    Ok(EventId {
        replica_id,
        counter,
    })
}

async fn insert_fact(
    db: &mut SqliteConnection,
    user: &User,
    id: &EventId,
    entity: LocalEntity<'_>,
    action: &(impl serde::Serialize + Sync),
) -> Result<(), DbError> {
    // These facts retain the original local observations. Missing births or deletion
    // references are legitimate legacy evidence, never a relaxed portable contract.
    let envelope = serde_json::to_string(&LocalHistory {
        entity,
        ancestry: "unknown",
        action,
    })
    .map_err(|_| DbError::Protocol("Invalid local history fact".into()))?;
    sqlx::query("INSERT INTO causal_events (user_id, writer_id, counter, encoding, envelope) VALUES (?, ?, ?, 'local_history_v1', ?)")
        .bind(user.id).bind(&id.replica_id).bind(id.counter).bind(envelope).execute(db).await?;

    Ok(())
}

pub(super) async fn record_group(
    db: &mut SqliteConnection,
    user: &User,
    global: &str,
    action: &GroupFact<'_>,
) -> Result<EventId, DbError> {
    let id = next_event(db, user).await?;
    let birth: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT group_birth_id FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?",
    )
    .bind(user.id).bind(global).fetch_optional(&mut *db).await?.flatten();
    insert_fact(
        db,
        user,
        &id,
        LocalEntity {
            kind: "group",
            global_id: global,
            birth_id: birth.as_deref(),
            local_incarnation: None,
        },
        action,
    )
    .await?;
    let event = sqlx::query("INSERT INTO group_causal_events (user_id, group_global_id, replica_id, counter) VALUES (?, ?, ?, ?)")
        .bind(user.id).bind(global).bind(&id.replica_id).bind(id.counter).execute(&mut *db).await?.last_insert_rowid();
    sqlx::query("INSERT INTO group_causal_parents (event_id, parent_id) SELECT ?, event_id FROM group_causal_frontier WHERE user_id = ? AND group_global_id = ?")
        .bind(event).bind(user.id).bind(global).execute(&mut *db).await?;
    sqlx::query("DELETE FROM group_causal_frontier WHERE user_id = ? AND group_global_id = ?")
        .bind(user.id)
        .bind(global)
        .execute(&mut *db)
        .await?;
    sqlx::query(
        "INSERT INTO group_causal_frontier (user_id, group_global_id, event_id) VALUES (?, ?, ?)",
    )
    .bind(user.id)
    .bind(global)
    .bind(event)
    .execute(db)
    .await?;

    Ok(id)
}

pub(super) async fn record_membership(
    db: &mut SqliteConnection,
    user: &User,
    model: &str,
    incarnation: &str,
    action: &MembershipFact<'_>,
) -> Result<EventId, DbError> {
    let id = next_event(db, user).await?;
    // Capture the exact live incarnation in the same transaction as its mutation.
    // A registry/global-id match alone cannot bind retained history after a purge.
    let birth: Option<String> = sqlx::query_scalar::<_, Option<String>>("SELECT model_birth_id FROM models WHERE model_user_id = ? AND model_unique_global_id = ? AND model_incarnation = ?")
        .bind(user.id).bind(model).bind(incarnation).fetch_optional(&mut *db).await?.flatten();
    insert_fact(
        db,
        user,
        &id,
        LocalEntity {
            kind: "model",
            global_id: model,
            birth_id: birth.as_deref(),
            local_incarnation: Some(incarnation),
        },
        action,
    )
    .await?;
    let event = sqlx::query("INSERT INTO membership_causal_events (user_id, model_global_id, model_incarnation, replica_id, counter, model_birth_id) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(user.id).bind(model).bind(incarnation).bind(&id.replica_id).bind(id.counter).bind(birth)
        .execute(&mut *db).await?.last_insert_rowid();
    sqlx::query("INSERT INTO membership_causal_parents (event_id, parent_id) SELECT ?, event_id FROM membership_causal_frontier WHERE user_id = ? AND model_global_id = ? AND model_incarnation = ?")
        .bind(event).bind(user.id).bind(model).bind(incarnation).execute(&mut *db).await?;
    sqlx::query("DELETE FROM membership_causal_frontier WHERE user_id = ? AND model_global_id = ? AND model_incarnation = ?")
        .bind(user.id).bind(model).bind(incarnation).execute(&mut *db).await?;
    sqlx::query("INSERT INTO membership_causal_frontier (user_id, model_global_id, model_incarnation, event_id) VALUES (?, ?, ?, ?)")
        .bind(user.id).bind(model).bind(incarnation).bind(event).execute(db).await?;

    Ok(id)
}
