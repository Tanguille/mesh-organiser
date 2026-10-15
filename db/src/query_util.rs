use sqlx::{QueryBuilder, Sqlite};

use crate::{DbError, db_context::DbContext, model::user::User};

/// Appends `(?, ?, …)` with one bind per value. Caller must prefix e.g. `column IN `.
pub fn push_in<'t, T>(builder: &mut QueryBuilder<Sqlite>, values: impl IntoIterator<Item = T>)
where
    T: sqlx::Encode<'t, Sqlite> + sqlx::Type<Sqlite>,
{
    builder.push("(");
    let mut separated = builder.separated(", ");
    for value in values {
        separated.push_bind(value);
    }
    builder.push(")");
}

/// Runs `UPDATE <table> SET <timestamp_column> = <timestamp> WHERE <id_column> IN (…) AND
/// <user_column> = <user_id>`. No-op when `ids` is empty. Table and column names are pushed
/// as raw SQL, so they must be trusted literals — never user input.
#[allow(clippy::too_many_arguments)]
pub async fn set_timestamp_column(
    db: &DbContext,
    table: &'static str,
    timestamp_column: &'static str,
    id_column: &'static str,
    user_column: &'static str,
    ids: &[i64],
    user_id: i64,
    timestamp: &str,
) -> Result<(), DbError> {
    if ids.is_empty() {
        return Ok(());
    }

    let mut query_builder = QueryBuilder::new("UPDATE ");
    query_builder.push(table);
    query_builder.push(" SET ");
    query_builder.push(timestamp_column);
    query_builder.push(" = ");
    query_builder.push_bind(timestamp);
    query_builder.push(" WHERE ");
    query_builder.push(id_column);
    query_builder.push(" IN ");
    push_in(&mut query_builder, ids);
    query_builder.push(" AND ");
    query_builder.push(user_column);
    query_builder.push(" = ");
    query_builder.push_bind(user_id);
    query_builder.build().execute(db).await?;

    Ok(())
}

/// Fails with `RowNotFound` unless every id in `ids` is a distinct row of `table` owned by
/// `user_id`, so missing, foreign and duplicate ids are all rejected. Ok when `ids` is empty.
/// Table and column names are pushed as raw SQL, so they must be trusted literals.
pub async fn ensure_owned(
    db: &DbContext,
    table: &'static str,
    id_column: &'static str,
    user_column: &'static str,
    ids: &[i64],
    user_id: i64,
) -> Result<(), DbError> {
    if ids.is_empty() {
        return Ok(());
    }

    let mut query_builder = QueryBuilder::new("SELECT COUNT(*) FROM ");
    query_builder.push(table);
    query_builder.push(" WHERE ");
    query_builder.push(id_column);
    query_builder.push(" IN ");
    push_in(&mut query_builder, ids);
    query_builder.push(" AND ");
    query_builder.push(user_column);
    query_builder.push(" = ");
    query_builder.push_bind(user_id);
    let owned: i64 = query_builder.build_query_scalar().fetch_one(db).await?;

    // Ids are primary keys, so a duplicated input id matches one row and the counts differ.
    if usize::try_from(owned).unwrap_or(0) != ids.len() {
        return Err(DbError::RowNotFound);
    }

    Ok(())
}

pub async fn ensure_labels_owned(db: &DbContext, user: &User, ids: &[i64]) -> Result<(), DbError> {
    ensure_owned(db, "labels", "label_id", "label_user_id", ids, user.id).await
}

pub async fn ensure_models_owned(db: &DbContext, user: &User, ids: &[i64]) -> Result<(), DbError> {
    ensure_owned(db, "models", "model_id", "model_user_id", ids, user.id).await
}
