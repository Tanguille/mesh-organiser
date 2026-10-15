use crate::{DbError, db_context::DbContext, model::blob::Blob, push_in, util::time_now};

pub async fn get_blobs(db: &DbContext) -> Result<Vec<Blob>, DbError> {
    sqlx::query_as("SELECT * FROM blobs").fetch_all(db).await
}

pub async fn get_blobs_via_ids(db: &DbContext, ids: Vec<i64>) -> Result<Vec<Blob>, DbError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut query_builder = sqlx::QueryBuilder::new("SELECT * FROM blobs WHERE blob_id IN ");
    push_in(&mut query_builder, &ids);

    query_builder.build_query_as().fetch_all(db).await
}

pub async fn get_blob_via_sha256(db: &DbContext, sha256: &str) -> Result<Option<Blob>, DbError> {
    sqlx::query_as("SELECT * FROM blobs WHERE blob_sha256 = ?")
        .bind(sha256)
        .fetch_optional(db)
        .await
}

pub async fn add_blob(
    db: &DbContext,
    sha256: &str,
    filetype: &str,
    size: i64,
    disk_path: Option<String>,
) -> Result<i64, DbError> {
    let now = time_now();

    let result = sqlx::query!(
        "INSERT INTO blobs (blob_sha256, blob_filetype, blob_size, blob_added, blob_path) VALUES (?, ?, ?, ?, ?)",
        sha256,
        filetype,
        size,
        now,
        disk_path,
    )
    .execute(db)
    .await?;

    Ok(result.last_insert_rowid())
}

/// Deletes every blob no model references and returns the deleted rows. One statement, so a
/// model added concurrently can't have its blob removed between a select and the delete.
pub async fn get_and_delete_dead_blobs(db: &DbContext) -> Result<Vec<Blob>, DbError> {
    sqlx::query_as(
        "DELETE FROM blobs
            WHERE blob_id NOT IN
                (SELECT DISTINCT model_blob_id
                    FROM models
                    WHERE model_blob_id IS NOT NULL)
            RETURNING *",
    )
    .fetch_all(db)
    .await
}
