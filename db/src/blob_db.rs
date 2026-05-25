use sqlx::{QueryBuilder, Row};

use crate::{DbError, db_context::DbContext, model::blob::Blob, push_in_i64, util::time_now};

pub async fn get_blobs(db: &DbContext) -> Result<Vec<Blob>, DbError> {
    let rows = sqlx::query!(
        "SELECT blob_id, blob_sha256, blob_filetype, blob_size, blob_added, blob_path FROM blobs"
    )
    .fetch_all(db)
    .await?;

    let mut blobs = Vec::with_capacity(rows.len());

    for row in rows {
        blobs.push(Blob {
            id: row.blob_id,
            sha256: row.blob_sha256,
            filetype: row.blob_filetype,
            size: row.blob_size,
            added: row.blob_added,
            disk_path: row.blob_path,
        });
    }

    Ok(blobs)
}

pub async fn get_blobs_via_ids(db: &DbContext, ids: Vec<i64>) -> Result<Vec<Blob>, DbError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut query_builder = QueryBuilder::new(
        "SELECT blob_id, blob_sha256, blob_filetype, blob_size, blob_added, blob_path FROM blobs WHERE blob_id IN ",
    );
    push_in_i64(&mut query_builder, &ids);
    let rows = query_builder.build().fetch_all(db).await?;

    let mut blobs = Vec::with_capacity(rows.len());

    for row in rows {
        blobs.push(Blob {
            id: row.get("blob_id"),
            sha256: row.get("blob_sha256"),
            filetype: row.get("blob_filetype"),
            size: row.get("blob_size"),
            added: row.get("blob_added"),
            disk_path: row.get("blob_path"),
        });
    }

    Ok(blobs)
}

pub async fn get_blob_via_sha256(db: &DbContext, sha256: &str) -> Result<Option<Blob>, DbError> {
    let row = sqlx::query!(
        "SELECT blob_id, blob_sha256, blob_filetype, blob_size, blob_added, blob_path FROM blobs WHERE blob_sha256 = ?",
        sha256
    )
    .fetch_optional(db)
    .await?;

    match row {
        Some(record) => Ok(Some(Blob {
            id: record.blob_id,
            sha256: record.blob_sha256,
            filetype: record.blob_filetype,
            size: record.blob_size,
            disk_path: record.blob_path,
            added: record.blob_added,
        })),
        None => Ok(None),
    }
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

pub async fn delete_blob(db: &DbContext, blob_id: i64) -> Result<(), DbError> {
    sqlx::query!("DELETE FROM blobs WHERE blob_id = ?", blob_id)
        .execute(db)
        .await?;

    Ok(())
}

pub async fn get_and_delete_dead_blobs(db: &DbContext) -> Result<Vec<Blob>, DbError> {
    let dead_blob_rows = sqlx::query!(
        "SELECT blob_id, blob_sha256, blob_filetype, blob_size, blob_added, blob_path FROM blobs
            WHERE blob_id NOT IN
                (SELECT DISTINCT model_blob_id
                    FROM models
                    WHERE model_blob_id IS NOT NULL)"
    )
    .fetch_all(db)
    .await?;

    let mut dead_blobs = Vec::with_capacity(dead_blob_rows.len());

    for row in dead_blob_rows {
        dead_blobs.push(Blob {
            id: row.blob_id,
            sha256: row.blob_sha256,
            filetype: row.blob_filetype,
            size: row.blob_size,
            added: row.blob_added,
            disk_path: row.blob_path,
        });
    }

    if !dead_blobs.is_empty() {
        let dead_ids: Vec<i64> = dead_blobs.iter().map(|blob| blob.id).collect();
        let mut query_builder = QueryBuilder::new("DELETE FROM blobs WHERE blob_id IN ");
        push_in_i64(&mut query_builder, &dead_ids);
        query_builder.build().execute(db).await?;
    }

    Ok(dead_blobs)
}

pub async fn get_blob_model_usage_count(db: &DbContext, blob_id: i64) -> Result<i64, DbError> {
    let row = sqlx::query!(
        "SELECT COUNT(*) as count FROM models WHERE model_blob_id = ?",
        blob_id
    )
    .fetch_one(db)
    .await?;

    Ok(row.count)
}
