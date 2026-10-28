use sqlx::QueryBuilder;

use crate::{
    DbError,
    db_context::DbContext,
    model::{share::Share, user::User},
    model_db, parse_concat_ids, random_hex_32, time_now,
};

#[allow(clippy::struct_field_names)]
struct ShareDbQuery {
    share_id: String,
    share_user_id: i64,
    share_created_at: String,
    share_name: String,
    share_model_ids: Option<String>,
}

impl ShareDbQuery {
    fn into_share(self) -> Share {
        Share {
            id: self.share_id,
            created_at: self.share_created_at,
            share_name: self.share_name,
            user_id: self.share_user_id,
            model_ids: self
                .share_model_ids
                .map_or_else(Vec::new, |ids| parse_concat_ids(&ids)),
        }
    }
}

// Both share reads leave trashed models out of `model_ids`: they join models as NULLs, which
// GROUP_CONCAT skips. Restoring a model puts it back in its shares.
pub async fn get_shares(db: &DbContext, user: &User) -> Result<Vec<Share>, DbError> {
    let rows = sqlx::query_as!(ShareDbQuery, "
        SELECT shares.share_id, share_user_id, share_created_at, share_name, GROUP_CONCAT(models.model_id) AS \"share_model_ids: String\"
        FROM shares
        LEFT JOIN shares_models ON shares.share_id = shares_models.share_id
        LEFT JOIN models ON shares_models.model_id = models.model_id AND models.model_deleted IS NULL
        WHERE share_user_id = ?
        GROUP BY shares.share_id
        ORDER BY share_created_at DESC
        ", user.id)
        .fetch_all(db)
        .await?;

    Ok(rows.into_iter().map(ShareDbQuery::into_share).collect())
}

pub async fn get_share_via_id(db: &DbContext, share_id: &str) -> Result<Share, DbError> {
    let row = sqlx::query_as!(ShareDbQuery, "
        SELECT shares.share_id, share_user_id, share_created_at, share_name, GROUP_CONCAT(models.model_id) AS \"share_model_ids: String\"
        FROM shares
        LEFT JOIN shares_models ON shares.share_id = shares_models.share_id
        LEFT JOIN models ON shares_models.model_id = models.model_id AND models.model_deleted IS NULL
        WHERE shares.share_id = ?
        GROUP BY shares.share_id
        ", share_id)
        .fetch_one(db)
        .await?;

    Ok(row.into_share())
}

pub async fn set_model_ids_on_share(
    db: &DbContext,
    user: &User,
    share_id: &str,
    model_ids: Vec<i64>,
) -> Result<(), DbError> {
    let expected = model_ids.len();
    let models = model_db::get_models_via_ids(db, user, model_ids).await?;

    if models.len() != expected {
        return Err(DbError::RowNotFound);
    }

    let share = sqlx::query!(
        "SELECT share_id FROM shares WHERE share_id = ? AND share_user_id = ?",
        share_id,
        user.id
    )
    .fetch_optional(db)
    .await?;

    if share.is_none() {
        return Err(DbError::RowNotFound);
    }

    sqlx::query!("DELETE FROM shares_models WHERE share_id = ?", share_id)
        .execute(db)
        .await?;

    // An empty VALUES list is invalid SQL; clearing the share stops at the DELETE above.
    if !models.is_empty() {
        let mut query_builder =
            QueryBuilder::new("INSERT INTO shares_models (share_id, model_id) ");
        query_builder.push_values(
            models.iter().map(|model| model.id),
            |mut builder, model_id| {
                builder.push_bind(share_id);
                builder.push_bind(model_id);
            },
        );
        query_builder.build().execute(db).await?;
    }

    Ok(())
}

pub async fn delete_share(db: &DbContext, user: &User, share_id: &str) -> Result<(), DbError> {
    sqlx::query!(
        "DELETE FROM shares WHERE share_id = ? AND share_user_id = ?",
        share_id,
        user.id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn create_share(
    db: &DbContext,
    user: &User,
    share_name: &str,
) -> Result<String, DbError> {
    let random_hex = random_hex_32();
    let now = time_now();

    sqlx::query!("
        INSERT INTO shares (share_id, share_user_id, share_created_at, share_name)VALUES (?, ?, ?, ?)",
        random_hex,
        user.id,
        now,
        share_name)
        .execute(db)
        .await?;

    Ok(random_hex)
}

pub async fn rename_share(
    db: &DbContext,
    user: &User,
    share_id: &str,
    new_name: &str,
) -> Result<(), DbError> {
    sqlx::query!(
        "
        UPDATE shares
        SET share_name = ?
        WHERE share_id = ? AND share_user_id = ?",
        new_name,
        share_id,
        user.id
    )
    .execute(db)
    .await?;

    Ok(())
}
