use indexmap::IndexMap;
use sqlx::QueryBuilder;

use crate::{
    DbError, LABELS,
    db_context::DbContext,
    ensure_owned,
    label_db::set_last_updated_on_labels,
    model::{label_keyword::LabelKeyword, user::User},
    timestamp_or_now,
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

pub async fn set_keywords_for_label(
    db: &DbContext,
    user: &User,
    label_id: i64,
    keywords: Vec<String>,
    update_timestamp: Option<&str>,
) -> Result<(), DbError> {
    ensure_owned(db, &LABELS, &[label_id], user).await?;

    sqlx::query!(
        "DELETE FROM label_keywords WHERE keyword_label_id = ?",
        label_id,
    )
    .execute(db)
    .await?;

    if !keywords.is_empty() {
        let mut query_builder =
            QueryBuilder::new("INSERT INTO label_keywords (keyword_name, keyword_label_id) ");
        query_builder.push_values(keywords.iter(), |mut builder, keyword| {
            builder.push_bind(keyword);
            builder.push_bind(label_id);
        });
        query_builder.build().execute(db).await?;
    }

    set_last_updated_on_labels(db, user, &[label_id], &timestamp_or_now(update_timestamp)).await
}
