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

/// A table whose rows each belong to one user. The names are pushed as raw SQL, so they must
/// be trusted literals — never user input.
pub struct OwnedTable {
    name: &'static str,
    id_column: &'static str,
    user_column: &'static str,
}

pub const LABELS: OwnedTable = OwnedTable {
    name: "labels",
    id_column: "label_id",
    user_column: "label_user_id",
};
pub const MODELS: OwnedTable = OwnedTable {
    name: "models",
    id_column: "model_id",
    user_column: "model_user_id",
};
pub const GROUPS: OwnedTable = OwnedTable {
    name: "models_group",
    id_column: "group_id",
    user_column: "group_user_id",
};
pub const RESOURCES: OwnedTable = OwnedTable {
    name: "resources",
    id_column: "resource_id",
    user_column: "resource_user_id",
};

impl OwnedTable {
    /// Appends `<id_column> IN (…) AND <user_column> = <user_id>`.
    fn push_owned_ids(&self, query_builder: &mut QueryBuilder<Sqlite>, ids: &[i64], user_id: i64) {
        query_builder.push(self.id_column);
        query_builder.push(" IN ");
        push_in(query_builder, ids);
        query_builder.push(" AND ");
        query_builder.push(self.user_column);
        query_builder.push(" = ");
        query_builder.push_bind(user_id);
    }
}

/// Sets `timestamp_column` to `timestamp` on the rows of `table` in `ids` owned by `user`.
/// No-op when `ids` is empty. `timestamp_column` is pushed as raw SQL, like the table names.
pub async fn set_timestamp_column(
    db: &DbContext,
    table: &OwnedTable,
    timestamp_column: &'static str,
    ids: &[i64],
    user: &User,
    timestamp: &str,
) -> Result<(), DbError> {
    if ids.is_empty() {
        return Ok(());
    }

    let mut query_builder = QueryBuilder::new("UPDATE ");
    query_builder.push(table.name);
    query_builder.push(" SET ");
    query_builder.push(timestamp_column);
    query_builder.push(" = ");
    query_builder.push_bind(timestamp);
    query_builder.push(" WHERE ");
    table.push_owned_ids(&mut query_builder, ids, user.id);
    query_builder.build().execute(db).await?;

    Ok(())
}

/// Fails with `RowNotFound` unless every id in `ids` is a distinct row of `table` owned by
/// `user`, so missing, foreign and duplicate ids are all rejected. Ok when `ids` is empty.
pub async fn ensure_owned(
    db: &DbContext,
    table: &OwnedTable,
    ids: &[i64],
    user: &User,
) -> Result<(), DbError> {
    if ids.is_empty() {
        return Ok(());
    }

    let mut query_builder = QueryBuilder::new("SELECT COUNT(*) FROM ");
    query_builder.push(table.name);
    query_builder.push(" WHERE ");
    table.push_owned_ids(&mut query_builder, ids, user.id);
    let owned: i64 = query_builder.build_query_scalar().fetch_one(db).await?;

    // Ids are primary keys, so a duplicated input id matches one row and the counts differ.
    if usize::try_from(owned).unwrap_or(0) != ids.len() {
        return Err(DbError::RowNotFound);
    }

    Ok(())
}
