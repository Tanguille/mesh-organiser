//! One allocator for every local journal path; authenticated accounts have their own clock.

use sqlx::SqliteConnection;

use crate::DbError;

pub async fn read(db: &mut SqliteConnection, user: i64) -> Result<(String, i64), DbError> {
    if let Some(clock) =
        sqlx::query_as("SELECT writer_id,counter FROM causal_account_writers WHERE user_id=?")
            .bind(user)
            .fetch_optional(&mut *db)
            .await?
    {
        return Ok(clock);
    }
    sqlx::query_as("SELECT replica_id,counter FROM group_causal_replica WHERE singleton=1")
        .fetch_one(db)
        .await
}

pub async fn set(
    db: &mut SqliteConnection,
    user: i64,
    writer: &str,
    previous: i64,
    next: i64,
) -> Result<(), DbError> {
    if next < 0 {
        return Err(DbError::Protocol("Invalid causal clock".into()));
    }
    let scoped: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM causal_account_writers WHERE user_id=?)")
            .bind(user)
            .fetch_one(&mut *db)
            .await?;
    let result = if scoped {
        sqlx::query("UPDATE causal_account_writers SET counter=? WHERE user_id=? AND writer_id=? AND counter=?").bind(next).bind(user).bind(writer).bind(previous).execute(db).await?
    } else {
        sqlx::query("UPDATE group_causal_replica SET counter=? WHERE singleton=1 AND replica_id=? AND counter=?").bind(next).bind(writer).bind(previous).execute(db).await?
    };
    if result.rows_affected() != 1 {
        return Err(DbError::Protocol("Invalid causal clock".into()));
    }

    Ok(())
}

pub async fn next(db: &mut SqliteConnection, user: i64) -> Result<(String, i64), DbError> {
    let (writer, previous) = read(db, user).await?;
    let next = previous
        .checked_add(1)
        .ok_or_else(|| DbError::Protocol("Causal clock exhausted".into()))?;
    set(db, user, &writer, previous, next).await?;

    Ok((writer, next))
}
