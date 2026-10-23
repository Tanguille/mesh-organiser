use indexmap::{IndexMap, IndexSet};
use sqlx::QueryBuilder;

use crate::{
    DbError, LABELS,
    db_context::DbContext,
    ensure_owned,
    model::{label_keyword::LabelKeyword, user::User},
    push_in, set_timestamp_column, timestamp_or_now,
};

pub async fn get_keywords_for_label(
    db: &DbContext,
    user: &User,
    label_id: i64,
) -> Result<Vec<LabelKeyword>, DbError> {
    let rows = sqlx::query!(
        "SELECT keyword_id, keyword_name FROM label_keywords JOIN labels ON label_keywords.keyword_label_id = labels.label_id WHERE keyword_label_id = ? AND label_user_id = ?",
        label_id,
        user.id
    )
    .fetch_all(db)
    .await?;

    let mut result: Vec<LabelKeyword> = Vec::new();
    for row in rows {
        result.push(LabelKeyword {
            id: row.keyword_id,
            name: row.keyword_name,
        });
    }

    Ok(result)
}

pub async fn get_all_keywords(
    db: &DbContext,
    user: &User,
) -> Result<IndexMap<i64, Vec<LabelKeyword>>, DbError> {
    let rows = sqlx::query!(
        "SELECT keyword_id, keyword_name, keyword_label_id FROM label_keywords JOIN labels ON label_keywords.keyword_label_id = labels.label_id WHERE label_user_id = ?",
        user.id
    )
    .fetch_all(db)
    .await?;

    let mut result: IndexMap<i64, Vec<LabelKeyword>> = IndexMap::new();

    for row in rows {
        let entry = result.entry(row.keyword_label_id).or_default();
        entry.push(LabelKeyword {
            id: row.keyword_id,
            name: row.keyword_name,
        });
    }

    Ok(result)
}

/// Replaces the keywords of `label_id` with `keywords`, dropping exact duplicates.
///
/// Keyword names are unique across all users' labels. The delete, insert and timestamp share
/// one transaction, so a keyword another label holds keeps the previous keywords instead of
/// leaving the label with none.
pub async fn set_keywords_for_label(
    db: &DbContext,
    user: &User,
    label_id: i64,
    keywords: Vec<String>,
    update_timestamp: Option<&str>,
) -> Result<(), DbError> {
    // Checked before the transaction so it opens with a write: a deferred SQLite transaction
    // that reads first can fail with SQLITE_BUSY when it later needs the write lock.
    ensure_owned(db, &LABELS, &[label_id], user).await?;
    // A duplicate would fail the unique constraint against its own copy.
    let keywords: IndexSet<String> = keywords.into_iter().collect();

    let mut transaction = db.begin().await?;
    sqlx::query!(
        "DELETE FROM label_keywords WHERE keyword_label_id = ?",
        label_id,
    )
    .execute(&mut *transaction)
    .await?;

    if !keywords.is_empty() {
        let mut query_builder =
            QueryBuilder::new("INSERT INTO label_keywords (keyword_name, keyword_label_id) ");
        query_builder.push_values(keywords.iter(), |mut builder, keyword| {
            builder.push_bind(keyword);
            builder.push_bind(label_id);
        });
        query_builder.build().execute(&mut *transaction).await?;
    }

    set_timestamp_column(
        &mut *transaction,
        &LABELS,
        "label_last_modified",
        &[label_id],
        user,
        &timestamp_or_now(update_timestamp),
    )
    .await?;

    transaction.commit().await?;

    Ok(())
}

/// The names among `keywords` that a label other than `label_id` holds, whoever owns it, since
/// names are unique across all users. Only names come back, never the label or its owner.
pub async fn get_keywords_held_elsewhere(
    db: &DbContext,
    keywords: &[String],
    label_id: i64,
) -> Result<Vec<String>, DbError> {
    if keywords.is_empty() {
        return Ok(Vec::new());
    }

    let mut query_builder =
        QueryBuilder::new("SELECT keyword_name FROM label_keywords WHERE keyword_name IN ");
    push_in(&mut query_builder, keywords);
    query_builder.push(" AND keyword_label_id != ");
    query_builder.push_bind(label_id);
    query_builder.push(" ORDER BY keyword_name");

    query_builder.build_query_scalar().fetch_all(db).await
}
