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

/// Runs `UPDATE <table> SET <ts_col> = <timestamp> WHERE <id_col> IN (…) AND <user_col> = <user_id>`.
/// No-op when `ids` is empty. Table/column names are interpolated into the SQL, so callers
/// must pass trusted literals — never user input.
#[allow(clippy::too_many_arguments)]
pub async fn set_timestamp_column(
    db: &DbContext,
    table: &str,
    ts_col: &str,
    id_col: &str,
    user_col: &str,
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
    query_builder.push(ts_col);
    query_builder.push(" = ");
    query_builder.push_bind(timestamp);
    query_builder.push(" WHERE ");
    query_builder.push(id_col);
    query_builder.push(" IN ");
    push_in_i64(&mut query_builder, ids);
    query_builder.push(" AND ");
    query_builder.push(user_col);
    query_builder.push(" = ");
    query_builder.push_bind(user_id);
    query_builder.build().execute(db).await?;

    Ok(())
}
