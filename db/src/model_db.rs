use itertools::Itertools;
use sqlx::{QueryBuilder, Row};
use strum::EnumString;

use crate::{
    DbError, GROUPS, LABELS, MAX_PAGE_SIZE, MODELS, PaginatedResponse,
    db_context::DbContext,
    ensure_owned, label_db,
    model::{Model, ModelFlags, blob::FileType, model_group::ModelGroupMeta, user::User},
    push_in, set_timestamp_column,
    util::{parse_concat_ids, random_hex_32, time_now, timestamp_or_now, validate_global_id},
};

#[derive(Debug, PartialEq, Eq, EnumString)]
pub enum ModelOrderBy {
    AddedAsc,
    AddedDesc,
    NameAsc,
    NameDesc,
    SizeAsc,
    SizeDesc,
    ModifiedAsc,
    ModifiedDesc,
}

impl ModelOrderBy {
    #[must_use]
    pub const fn to_sql(&self) -> &'static str {
        match self {
            Self::AddedAsc => "model_added ASC",
            Self::AddedDesc => "model_added DESC",
            Self::NameAsc => "model_name ASC",
            Self::NameDesc => "model_name DESC",
            Self::SizeAsc => "blob_size ASC",
            Self::SizeDesc => "blob_size DESC",
            Self::ModifiedAsc => "model_last_modified ASC",
            Self::ModifiedDesc => "model_last_modified DESC",
        }
    }
}

pub struct ModelFilterOptions {
    pub model_ids: Option<Vec<i64>>,
    pub group_ids: Option<Vec<i64>>,
    pub label_ids: Option<Vec<i64>>,
    pub order_by: Option<ModelOrderBy>,
    pub text_search: Option<String>,
    pub model_flags: Option<ModelFlags>,
    /// Restrict to blobs of these types (zipped or not). `None` = any; `Some(&[])` = nothing.
    pub file_types: Option<Vec<FileType>>,
    /// `false` lists only live models, `true` only the ones in the trash.
    pub trashed: bool,
    pub page: u32,
    pub page_size: u32,
}

impl Default for ModelFilterOptions {
    // `page`/`page_size` must default to a valid pagination window, not 0: the query uses
    // `LIMIT {page_size}`, so a derived `page_size == 0` would silently return no rows.
    // Mirror the codebase's "fetch all" convention (page 1, MAX_PAGE_SIZE).
    fn default() -> Self {
        Self {
            model_ids: None,
            group_ids: None,
            label_ids: None,
            order_by: None,
            text_search: None,
            model_flags: None,
            file_types: None,
            trashed: false,
            page: 1,
            page_size: MAX_PAGE_SIZE,
        }
    }
}

/// Returns a page of the user's models matching all supplied filters.
///
/// File types match stored extensions case-insensitively and include equivalent zipped and
/// unzipped spellings. An empty ID or file-type list matches no models. Trashed models are left
/// out unless `options.trashed` asks for only those, which then default to newest-deleted first.
///
/// # Errors
///
/// Returns an error if the models or their labels cannot be queried.
pub async fn get_models(
    db: &DbContext,
    user: &User,
    options: ModelFilterOptions,
) -> Result<PaginatedResponse<Model>, DbError> {
    let page = options.page;
    let page_size = options.page_size.min(MAX_PAGE_SIZE);
    let items = query_models(db, user, options, Some(page_size)).await?;

    Ok(PaginatedResponse {
        page,
        page_size,
        items,
    })
}

/// Every model matching `options`, in one query; `options.page`/`page_size` are ignored.
pub async fn get_all_models(
    db: &DbContext,
    user: &User,
    options: ModelFilterOptions,
) -> Result<Vec<Model>, DbError> {
    query_models(db, user, options, None).await
}

/// The query behind [`get_models`] and [`get_all_models`]: one page of `page_size` rows at
/// `options.page`, or every match when `page_size` is `None`.
#[allow(clippy::too_many_lines)]
async fn query_models(
    db: &DbContext,
    user: &User,
    options: ModelFilterOptions,
    page_size: Option<u32>,
) -> Result<Vec<Model>, DbError> {
    if options.model_ids.as_ref().is_some_and(Vec::is_empty)
        || options.group_ids.as_ref().is_some_and(Vec::is_empty)
        || options.label_ids.as_ref().is_some_and(Vec::is_empty)
        || options.file_types.as_ref().is_some_and(Vec::is_empty)
    {
        return Ok(Vec::new());
    }

    let mut query_builder = QueryBuilder::new(
        "SELECT models.model_id, model_name, model_url, model_desc, model_added, model_flags, model_unique_global_id, model_last_modified, model_deleted,
				blob_id, blob_sha256, blob_filetype, blob_size, blob_path, blob_added,
                GROUP_CONCAT(labels.label_id) AS label_ids,
                models_group.group_id, group_name, group_created, group_resource_id, group_unique_global_id, group_last_modified
         FROM models
         LEFT JOIN models_labels ON models.model_id = models_labels.model_id
         LEFT JOIN labels ON models_labels.label_id = labels.label_id
         LEFT JOIN models_group ON models.model_group_id = models_group.group_id
		 INNER JOIN blobs ON models.model_blob_id = blobs.blob_id
         WHERE models.model_user_id = ",
    );
    query_builder.push_bind(user.id);
    query_builder.push(if options.trashed {
        " AND models.model_deleted IS NOT NULL"
    } else {
        " AND models.model_deleted IS NULL"
    });

    if let Some(model_ids) = &options.model_ids {
        query_builder.push(" AND models.model_id IN ");
        push_in(&mut query_builder, model_ids);
    }

    if let Some(group_ids) = &options.group_ids {
        query_builder.push(" AND group_id IN ");
        push_in(&mut query_builder, group_ids);
    }

    if let Some(label_ids) = &options.label_ids {
        query_builder.push(" AND labels.label_id IN ");
        push_in(&mut query_builder, label_ids);
    }

    if let Some(model_flags) = options.model_flags {
        query_builder.push(" AND (models.model_flags & ");
        query_builder.push_bind(i64::from(model_flags.bits()));
        query_builder.push(") = ");
        query_builder.push_bind(i64::from(model_flags.bits()));
    }

    if let Some(file_types) = &options.file_types {
        query_builder.push(" AND LOWER(blob_filetype) IN ");
        push_in(
            &mut query_builder,
            file_types.iter().flat_map(FileType::storage_extensions),
        );
    }

    if let Some(text_search) = options.text_search {
        let pattern = format!("%{text_search}%");
        query_builder.push(" AND (model_name LIKE ");
        query_builder.push_bind(pattern.clone());
        query_builder.push(" OR model_desc LIKE ");
        query_builder.push_bind(pattern.clone());
        query_builder.push(" OR group_name LIKE ");
        query_builder.push_bind(pattern);
        query_builder.push(")");
    }

    // The trailing `model_id` makes the order total, so consecutive pages neither skip nor
    // repeat models that tie.
    query_builder.push(" GROUP BY models.model_id ORDER BY ");
    if let Some(order_by_sql) = options
        .order_by
        .as_ref()
        .map(ModelOrderBy::to_sql)
        .or_else(|| options.trashed.then_some("model_deleted DESC"))
    {
        query_builder.push(format!("{order_by_sql}, "));
    }
    query_builder.push("models.model_id");

    if let Some(page_size) = page_size {
        let offset = (i64::from(options.page) - 1) * i64::from(page_size);
        query_builder.push(format!(" LIMIT {page_size} OFFSET {offset}"));
    }

    let query = query_builder.build();
    let rows = query.fetch_all(db).await?;
    let mut models = Vec::with_capacity(rows.len());

    let min_labels_map: indexmap::IndexMap<_, _> = label_db::get_labels_min(db, user)
        .await?
        .into_iter()
        .map(|l| (l.id, l))
        .collect();

    for row in rows {
        models.push(Model {
            id: row.get("model_id"),
            name: row.get("model_name"),
            last_modified: row.get("model_last_modified"),
            blob: sqlx::FromRow::from_row(&row)?,
            link: row.get("model_url"),
            description: row.get("model_desc"),
            added: row.get("model_added"),
            group: row
                .get::<Option<i64>, _>("group_id")
                .map(|id| ModelGroupMeta {
                    id,
                    name: row.get("group_name"),
                    created: row.get("group_created"),
                    resource_id: row.get("group_resource_id"),
                    unique_global_id: row.get("group_unique_global_id"),
                    last_modified: row.get("group_last_modified"),
                }),
            labels: row
                .get::<Option<String>, _>("label_ids")
                .map_or_else(Vec::new, |label_ids| {
                    parse_concat_ids(&label_ids)
                        .iter()
                        .filter_map(|id| min_labels_map.get(id).cloned())
                        .collect()
                }),
            flags: ModelFlags::from_bits(
                u32::try_from(row.get::<i64, _>("model_flags")).unwrap_or(0),
            )
            .unwrap_or_else(ModelFlags::empty),
            unique_global_id: row.get("model_unique_global_id"),
            deleted: row.get("model_deleted"),
        });
    }

    Ok(models)
}

pub async fn get_models_via_ids(
    db: &DbContext,
    user: &User,
    ids: Vec<i64>,
) -> Result<Vec<Model>, DbError> {
    let options = ModelFilterOptions {
        model_ids: Some(ids),
        ..Default::default()
    };

    let paginated_response = get_models(db, user, options).await?;
    Ok(paginated_response.items)
}

/// Fetches a single model by id, returning `None` when it does not exist for the user.
pub async fn get_model_via_id(
    db: &DbContext,
    user: &User,
    id: i64,
) -> Result<Option<Model>, DbError> {
    Ok(get_models_via_ids(db, user, vec![id])
        .await?
        .into_iter()
        .next())
}

pub async fn add_model(
    db: &DbContext,
    user: &User,
    name: &str,
    blob_id: i64,
    link: Option<&str>,
    update_timestamp: Option<&str>,
) -> Result<i64, DbError> {
    let now = time_now();
    let timestamp = update_timestamp.unwrap_or(&now);
    let hex = random_hex_32();

    let result = sqlx::query!(
        "INSERT INTO models (model_name, model_blob_id, model_added, model_url, model_user_id, model_unique_global_id, model_last_modified)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        name,
        blob_id,
        now,
        link,
        user.id,
        hex,
        timestamp,
    )
    .execute(db)
    .await?;

    Ok(result.last_insert_rowid())
}

#[allow(clippy::too_many_arguments)]
pub async fn edit_model(
    db: &DbContext,
    user: &User,
    id: i64,
    name: &str,
    link: Option<&str>,
    description: Option<&str>,
    flags: ModelFlags,
    update_timestamp: Option<&str>,
    global_id: Option<&str>,
) -> Result<(), DbError> {
    validate_global_id(global_id)?;
    let timestamp = timestamp_or_now(update_timestamp);
    let flags = i64::from(flags.bits());
    sqlx::query!(
        "UPDATE models SET model_name = ?, model_url = ?, model_desc = ?, model_flags = ?, model_last_modified = ?, model_unique_global_id = COALESCE(?, model_unique_global_id) WHERE model_id = ? AND model_user_id = ?",
        name,
        link,
        description,
        flags,
        timestamp,
        global_id,
        id,
        user.id
    )
    .execute(db)
    .await?;

    Ok(())
}

/// Moves the user's live models among `ids` to the trash.
///
/// `model_last_modified` is left alone on purpose: sync then sees the model missing on this side
/// with a timestamp older than the last sync, and trashes the other side's copy too.
pub async fn trash_models(db: &DbContext, user: &User, ids: &[i64]) -> Result<(), DbError> {
    if ids.is_empty() {
        return Ok(());
    }

    let mut query_builder = QueryBuilder::new("UPDATE models SET model_deleted = ");
    query_builder.push_bind(time_now());
    query_builder.push(" WHERE model_deleted IS NULL AND model_user_id = ");
    query_builder.push_bind(user.id);
    query_builder.push(" AND model_id IN ");
    push_in(&mut query_builder, ids);
    query_builder.build().execute(db).await?;

    Ok(())
}

/// Takes the user's trashed models among `ids` out of the trash, for a restore the user asked for.
/// Ids of live models are left as they are.
///
/// The restored models, their groups and their labels all get the same fresh last-modified
/// timestamp, so sync sees them as newer than the last sync: it re-uploads or re-downloads the
/// model, which restores the other side's trashed copy via the import dedupe, and keeps the
/// group and labels even when the other side no longer has the model (purged, or hard-deleted
/// by an older version).
///
/// # Errors
///
/// `RowNotFound`, restoring nothing, unless every id ends up a live model of the user: an
/// unknown or foreign id, or one a concurrent trash purge deleted first.
pub async fn restore_models(db: &DbContext, user: &User, ids: &[i64]) -> Result<(), DbError> {
    restore(db, user, ids, true).await
}

/// Like [`restore_models`], but the groups and labels keep their last-modified timestamps.
///
/// Ids of live models are left as they are, so the import dedupe can pass whichever row it
/// matched. The dedupe is how sync carries a restore over, and sync stamps the model with the
/// other side's timestamp right after. Stamping the groups and labels here would make this
/// side's possibly stale group and label state look newest and revert the other side's edits
/// on the next sync.
///
/// # Errors
///
/// As [`restore_models`].
pub async fn untrash_models(db: &DbContext, user: &User, ids: &[i64]) -> Result<(), DbError> {
    restore(db, user, ids, false).await
}

async fn restore(
    db: &DbContext,
    user: &User,
    ids: &[i64],
    stamp_relations: bool,
) -> Result<(), DbError> {
    let ids: Vec<i64> = ids.iter().copied().unique().collect();
    if ids.is_empty() {
        return Ok(());
    }

    let timestamp = time_now();
    let mut transaction = db.begin().await?;

    let mut query_builder =
        QueryBuilder::new("UPDATE models SET model_deleted = NULL, model_last_modified = ");
    query_builder.push_bind(timestamp.as_str());
    query_builder.push(" WHERE model_deleted IS NOT NULL AND model_user_id = ");
    query_builder.push_bind(user.id);
    query_builder.push(" AND model_id IN ");
    push_in(&mut query_builder, &ids);
    query_builder.push(" RETURNING model_id, model_group_id");
    let restored: Vec<(i64, Option<i64>)> = query_builder
        .build_query_as()
        .fetch_all(&mut *transaction)
        .await?;

    // The update holds the write lock and restored every owned trashed row among `ids`, so
    // existence alone now tells a live row from one purged under us.
    ensure_owned(&mut *transaction, &MODELS, &ids, user).await?;
    if !stamp_relations || restored.is_empty() {
        transaction.commit().await?;

        return Ok(());
    }

    let group_ids: Vec<i64> = restored
        .iter()
        .filter_map(|&(_, group_id)| group_id)
        .unique()
        .collect();
    set_timestamp_column(
        &mut *transaction,
        &GROUPS,
        "group_last_modified",
        &group_ids,
        user,
        &timestamp,
    )
    .await?;

    let mut query_builder =
        QueryBuilder::new("SELECT DISTINCT label_id FROM models_labels WHERE model_id IN ");
    push_in(
        &mut query_builder,
        restored.iter().map(|&(model_id, _)| model_id),
    );
    let label_ids: Vec<i64> = query_builder
        .build_query_scalar()
        .fetch_all(&mut *transaction)
        .await?;
    set_timestamp_column(
        &mut *transaction,
        &LABELS,
        "label_last_modified",
        &label_ids,
        user,
        &timestamp,
    )
    .await?;

    transaction.commit().await?;

    Ok(())
}

/// Permanently deletes every user's models trashed more than `retention_days` ago and returns how
/// many rows went.
pub async fn purge_trashed_models(db: &DbContext, retention_days: u32) -> Result<u64, DbError> {
    // The cutoff is formatted like `time_now` so the string comparison orders correctly. A
    // retention too long for SQLite's date range yields NULL, which expires nothing.
    let result = sqlx::query!(
        "DELETE FROM models WHERE model_deleted < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-' || ? || ' days')",
        retention_days
    )
    .execute(db)
    .await?;

    Ok(result.rows_affected())
}

// TODO: Can we make a get model via sha256?
/// Includes trashed models: the import dedupe restores them, and blob downloads only need ownership.
pub async fn get_model_id_via_sha256(
    db: &DbContext,
    user: &User,
    sha256: &str,
) -> Result<Option<i64>, DbError> {
    let row = sqlx::query!(
        "SELECT model_id FROM models INNER JOIN blobs ON models.model_blob_id = blobs.blob_id WHERE blob_sha256 = ? AND model_user_id = ?",
        sha256,
        user.id
    )
    .fetch_optional(db)
    .await?;

    Ok(row.map(|r| r.model_id))
}

/// Resolve model IDs for a list of blob SHA256 hashes in one query.
///
/// Returns IDs in the same order as `sha256s`. Fails with `DbError` if the number of found
/// models does not match (e.g. unknown or inaccessible sha256). Trashed models count, like in
/// [`get_model_id_via_sha256`].
pub async fn get_model_ids_via_sha256s(
    db: &DbContext,
    user: &User,
    sha256s: &[String],
) -> Result<Vec<i64>, DbError> {
    if sha256s.is_empty() {
        return Ok(Vec::new());
    }

    let mut query_builder = QueryBuilder::new(
        "SELECT blob_sha256, model_id FROM models INNER JOIN blobs ON models.model_blob_id = blobs.blob_id WHERE blob_sha256 IN ",
    );
    push_in(&mut query_builder, sha256s);
    query_builder.push(" AND model_user_id = ");
    query_builder.push_bind(user.id);
    let sha256_to_id: std::collections::HashMap<String, i64> = query_builder
        .build_query_as::<(String, i64)>()
        .fetch_all(db)
        .await?
        .into_iter()
        .collect();

    sha256s
        .iter()
        .map(|sha256| sha256_to_id.get(sha256).copied())
        .collect::<Option<Vec<_>>>()
        .ok_or(DbError::RowNotFound)
}

pub async fn get_model_count(
    db: &DbContext,
    user: &User,
    flags: Option<ModelFlags>,
) -> Result<usize, DbError> {
    let count = match flags {
        Some(f) => {
            let bits = i64::from(f.bits());
            sqlx::query!(
                "SELECT COUNT(*) as count FROM models WHERE model_user_id = ? AND model_deleted IS NULL AND (models.model_flags & ?) = ?",
                user.id,
                bits,
                bits
            )
            .fetch_one(db)
            .await?.count
        }
        None => {
            sqlx::query!(
                "SELECT COUNT(*) as count FROM models WHERE model_user_id = ? AND model_deleted IS NULL",
                user.id
            )
            .fetch_one(db)
            .await?
            .count
        }
    };

    Ok(usize::try_from(count).unwrap_or(0))
}

pub struct ModelSizeResult {
    pub total_size: i64,
    pub blob_sha256: Vec<String>,
}

pub async fn get_size_of_models(db: &DbContext, user: &User) -> Result<ModelSizeResult, DbError> {
    let row = sqlx::query!(
        "SELECT SUM(blob_size) as \"total_size: i64\", GROUP_CONCAT(blob_sha256) as \"blob_sha256: String\" FROM models JOIN blobs ON models.model_blob_id = blobs.blob_id WHERE model_user_id = ? AND model_deleted IS NULL",
        user.id
    )
    .fetch_one(db)
    .await?;

    Ok(ModelSizeResult {
        total_size: row.total_size.unwrap_or(0),
        blob_sha256: row.blob_sha256.map_or(Vec::new(), |f| {
            f.split(',').map(ToString::to_string).collect()
        }),
    })
}
