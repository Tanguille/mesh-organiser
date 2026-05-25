use sqlx::{QueryBuilder, Sqlite};

/// Appends `(?, ?, …)` with one bind per id. Caller must prefix e.g. `column IN `.
pub fn push_in_i64(builder: &mut QueryBuilder<Sqlite>, ids: &[i64]) {
    builder.push("(");
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(*id);
    }
    builder.push(")");
}
