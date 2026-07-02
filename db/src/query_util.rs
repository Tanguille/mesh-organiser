use sqlx::{QueryBuilder, Sqlite};

use crate::{DbError, db_context::DbContext};

/// Appends `(?, ?, …)` with one bind per id. Caller must prefix e.g. `column IN `.
pub fn push_in_i64(builder: &mut QueryBuilder<Sqlite>, ids: &[i64]) {
    builder.push("(");
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(*id);
    }
    builder.push(")");
}

/// Identifies the table and column names for a `set_timestamp_column` call.
/// All fields must be trusted static literals — never user input.
pub struct TimestampSchema {
    pub table: &'static str,
    pub ts_col: &'static str,
    pub id_col: &'static str,
    pub user_col: &'static str,
}

/// Runs `UPDATE <table> SET <ts_col> = <timestamp> WHERE <id_col> IN (…) AND <user_col> = <user_id>`.
/// No-op when `ids` is empty.
pub async fn set_timestamp_column(
    db: &DbContext,
    schema: TimestampSchema,
    ids: &[i64],
    user_id: i64,
    timestamp: &str,
) -> Result<(), DbError> {
    if ids.is_empty() {
        return Ok(());
    }

    let mut query_builder = QueryBuilder::new("UPDATE ");
    query_builder.push(schema.table);
    query_builder.push(" SET ");
    query_builder.push(schema.ts_col);
    query_builder.push(" = ");
    query_builder.push_bind(timestamp);
    query_builder.push(" WHERE ");
    query_builder.push(schema.id_col);
    query_builder.push(" IN ");
    push_in_i64(&mut query_builder, ids);
    query_builder.push(" AND ");
    query_builder.push(schema.user_col);
    query_builder.push(" = ");
    query_builder.push_bind(user_id);
    query_builder.build().execute(db).await?;

    Ok(())
}
