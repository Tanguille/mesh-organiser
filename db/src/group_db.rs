use std::collections::HashSet;

use indexmap::IndexMap;
use itertools::Itertools;
use sqlx::{QueryBuilder, Row, SqliteConnection};

pub use crate::group_sync::{
    AssignmentBatchResult, AssignmentChange, GroupLifecycleChange, GroupPrecondition, KeepResource,
    apply_assignment_batch, apply_group_sync_batch,
};

use crate::{
    DbError, GROUPS, MAX_PAGE_SIZE, PaginatedResponse,
    db_context::DbContext,
    model::{
        Model, ModelFlags,
        model_group::{ModelGroup, ModelGroupMeta},
        resource::ResourceMeta,
        user::User,
    },
    model_db::{self, ModelFilterOptions},
    push_in, random_hex_32, set_timestamp_column, timestamp_or_now,
    util::time_now,
};

#[derive(Debug, PartialEq, Eq, strum::EnumString)]
pub enum GroupOrderBy {
    CreatedAsc,
    CreatedDesc,
    NameAsc,
    NameDesc,
    ModifiedAsc,
    ModifiedDesc,
}

pub struct GroupFilterOptions {
    pub model_ids: Option<Vec<i64>>,
    pub group_ids: Option<Vec<i64>>,
    pub label_ids: Option<Vec<i64>>,
    pub order_by: Option<GroupOrderBy>,
    pub text_search: Option<String>,
    /// Keep only groups containing at least one model of these types; see `ModelFilterOptions`.
    pub file_types: Option<Vec<crate::model::blob::FileType>>,
    /// 1-based page number. `Default` returns 1; values of 0 are treated as 1 by `get_groups`
    /// to keep the function panic-free even if a caller forgets to override the field.
    pub page: u32,
    /// Items per page. `Default` returns `MAX_PAGE_SIZE`; values of 0 are treated as
    /// `MAX_PAGE_SIZE` by `get_groups` for the same reason.
    pub page_size: u32,
    pub include_ungrouped_models: bool,
    pub allow_incomplete_groups: bool,
    /// When true and filters are applied, keep groups that only contain matching models
    /// (don't re-fetch full groups). When false, re-fetch so each group shows all its models.
    pub split_incomplete_groups: bool,
}

impl Default for GroupFilterOptions {
    // `page`/`page_size` must default to a valid pagination window, not 0: the in-memory
    // pagination below does `.take(page_size)`, so a derived `page_size == 0` would silently
    // return no groups. Mirror the codebase's "fetch all" convention (page 1, MAX_PAGE_SIZE).
    fn default() -> Self {
        Self {
            model_ids: None,
            group_ids: None,
            label_ids: None,
            order_by: None,
            text_search: None,
            file_types: None,
            page: 1,
            page_size: MAX_PAGE_SIZE,
            include_ungrouped_models: false,
            allow_incomplete_groups: false,
            split_incomplete_groups: false,
        }
    }
}

/// Builds group list from a flat model list. Main cost is typically the upstream fetch of all models.
fn convert_model_list_to_groups(
    models: Vec<Model>,
    include_ungrouped_models: bool,
    group_resource_map: &IndexMap<i64, ResourceMeta>,
) -> Vec<ModelGroup> {
    let mut index_map: IndexMap<i64, ModelGroup> = IndexMap::new();

    for mut model in models {
        let group_meta = if let Some(group) = model.group.take() {
            group
        } else {
            if !include_ungrouped_models {
                continue;
            }

            ModelGroupMeta {
                id: -model.id,
                name: model.name.clone(),
                created: model.added.clone(),
                resource_id: None,
                unique_global_id: String::default(),
                last_modified: model.last_modified.clone(),
            }
        };

        let group = index_map.entry(group_meta.id).or_insert({
            let mut meta = ModelGroup::from_meta(group_meta);

            if let Some(resource_meta) = group_resource_map.get(&meta.meta.id) {
                meta.resource = Some(resource_meta.clone());
            }

            meta
        });

        group.flags |= ModelFlags::from_bits_truncate(model.flags.bits());

        for label in &model.labels {
            if !group.labels.iter().any(|existing| existing.id == label.id) {
                group.labels.push(label.clone());
            }
        }

        group.models.push(model);
    }

    index_map.into_values().collect()
}

fn append_empty_groups(groups: &mut Vec<ModelGroup>, empty_groups: Vec<ModelGroup>) {
    let existing_ids: HashSet<i64> = groups.iter().map(|group| group.meta.id).collect();
    groups.extend(
        empty_groups
            .into_iter()
            .filter(|group| !existing_ids.contains(&group.meta.id)),
    );
}

async fn get_empty_groups(
    db: &DbContext,
    user: &User,
    options: &GroupFilterOptions,
    group_resource_map: &IndexMap<i64, ResourceMeta>,
) -> Result<Vec<ModelGroup>, DbError> {
    // A group with only trashed models stays hidden until one is restored.
    if options.label_ids.is_some()
        || options.model_ids.is_some()
        || options.file_types.is_some()
        || options.group_ids.as_ref().is_some_and(Vec::is_empty)
    {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::new(
        "SELECT group_id, group_name, group_created, group_last_modified, group_resource_id, group_unique_global_id FROM models_group WHERE group_deleted_at IS NULL AND group_user_id = ",
    );
    query.push_bind(user.id);
    query.push(
        " AND group_id NOT IN (SELECT model_group_id FROM models WHERE model_group_id IS NOT NULL)",
    );
    if let Some(group_ids) = &options.group_ids {
        query.push(" AND group_id IN ");
        push_in(&mut query, group_ids);
    }
    if let Some(text) = &options.text_search {
        query.push(" AND group_name LIKE ");
        query.push_bind(format!("%{text}%"));
    }
    let mut groups = Vec::new();
    for row in query.build().fetch_all(db).await? {
        let meta = ModelGroupMeta {
            id: row.try_get("group_id")?,
            name: row.try_get("group_name")?,
            created: row.try_get("group_created")?,
            last_modified: row.try_get("group_last_modified")?,
            resource_id: row.try_get("group_resource_id")?,
            unique_global_id: row.try_get("group_unique_global_id")?,
        };
        let resource = group_resource_map.get(&meta.id).cloned();
        let mut group = ModelGroup::from_meta(meta);
        group.resource = resource;
        groups.push(group);
    }

    Ok(groups)
}

/// Returns a page of groups whose models match the supplied filters, including empty groups.
///
/// Unless incomplete groups are allowed, model-level filters select groups but each returned
/// group is repopulated with all of its models. Ungrouped models are represented as individual
/// groups when requested.
///
/// # Errors
///
/// Returns an error if the groups or their related models and resources cannot be queried.
pub async fn get_groups(
    db: &DbContext,
    user: &User,
    options: GroupFilterOptions,
) -> Result<PaginatedResponse<ModelGroup>, DbError> {
    let filtered_on_labels = options.label_ids.is_some();
    let filtered_on_text = options.text_search.is_some();
    let filtered_on_models = options.model_ids.is_some();
    let filtered_on_file_types = options.file_types.is_some();

    let group_resource_map = crate::resource_db::get_group_id_to_resource_map(db, user).await?;

    let empty_groups = get_empty_groups(db, user, &options, &group_resource_map).await?;

    // Pagination happens in-memory below, so fetch every match.
    let models = model_db::get_all_models(
        db,
        user,
        ModelFilterOptions {
            model_ids: options.model_ids,
            group_ids: options.group_ids,
            label_ids: options.label_ids,
            text_search: options.text_search,
            file_types: options.file_types,
            ..Default::default()
        },
    )
    .await?;

    let mut groups = convert_model_list_to_groups(
        models,
        options.include_ungrouped_models,
        &group_resource_map,
    );

    // When filters were applied, we may have only a subset of each group's models. Re-fetch full
    // groups unless the caller allows incomplete groups or wants to keep filtered groups as-is.
    let re_fetch_full_groups =
        (filtered_on_labels || filtered_on_text || filtered_on_models || filtered_on_file_types)
            && !options.allow_incomplete_groups
            && !options.split_incomplete_groups;

    if re_fetch_full_groups {
        let group_ids: Vec<i64> = groups
            .iter()
            .filter(|group| group.meta.id >= 0)
            .map(|group| group.meta.id)
            .collect();
        let fake_models: Vec<ModelGroup> = groups
            .into_iter()
            .filter(|group| group.meta.id < 0)
            .collect();

        let models = model_db::get_all_models(
            db,
            user,
            ModelFilterOptions {
                group_ids: Some(group_ids),
                ..Default::default()
            },
        )
        .await?;

        groups = convert_model_list_to_groups(models, false, &group_resource_map);
        groups.extend(fake_models);
    }

    append_empty_groups(&mut groups, empty_groups);

    match options.order_by.unwrap_or(GroupOrderBy::CreatedDesc) {
        GroupOrderBy::CreatedAsc => {
            groups.sort_by(|left, right| left.meta.created.cmp(&right.meta.created));
        }
        GroupOrderBy::CreatedDesc => {
            groups.sort_by(|left, right| right.meta.created.cmp(&left.meta.created));
        }
        GroupOrderBy::NameAsc => groups.sort_by(|left, right| left.meta.name.cmp(&right.meta.name)),
        GroupOrderBy::NameDesc => {
            groups.sort_by(|left, right| right.meta.name.cmp(&left.meta.name));
        }
        GroupOrderBy::ModifiedAsc => {
            groups.sort_by(|left, right| left.meta.last_modified.cmp(&right.meta.last_modified));
        }
        GroupOrderBy::ModifiedDesc => {
            groups.sort_by(|left, right| right.meta.last_modified.cmp(&left.meta.last_modified));
        }
    }

    // `page` is 1-based, so 0 must be treated as 1; a
    // `page_size` of 0 would make `.take(0)` silently return an empty list.
    // `page` can be any u32 from the Tauri command, so saturate: a page past the end is empty.
    let page_size = options.page_size.clamp(1, MAX_PAGE_SIZE);
    let offset = (options.page.max(1) - 1).saturating_mul(page_size) as usize;

    Ok(PaginatedResponse {
        items: groups
            .into_iter()
            .skip(offset)
            .take(page_size as usize)
            .collect(),
        page: options.page,
        page_size,
    })
}

type ModelAssignmentRow = (i64, Option<i64>, String, String, Option<String>);

async fn ensure_unambiguous_move(
    db: &mut SqliteConnection,
    user: &User,
    model: &str,
    source: Option<&str>,
    target: Option<&str>,
) -> Result<(), DbError> {
    let model_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models WHERE model_user_id = ? AND model_unique_global_id = ?",
    )
    .bind(user.id)
    .bind(model)
    .fetch_one(&mut *db)
    .await?;
    if model_count != 1 {
        return Err(DbError::Protocol(
            "Invalid or ambiguous model sync identity".into(),
        ));
    }
    for global in source.into_iter().chain(target) {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?",
        )
        .bind(user.id)
        .bind(global)
        .fetch_one(&mut *db)
        .await?;
        if global.is_empty() || count != 1 {
            return Err(DbError::Protocol(
                "Invalid or ambiguous group sync identity".into(),
            ));
        }
    }

    Ok(())
}

#[allow(clippy::too_many_lines)]
pub async fn set_group_id_on_models(
    db: &DbContext,
    user: &User,
    group_id: Option<i64>,
    model_ids: Vec<i64>,
    update_timestamp: Option<&str>,
) -> Result<(), DbError> {
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    let mut select = QueryBuilder::new(
        "SELECT m.model_id, m.model_group_id, m.model_unique_global_id, m.model_incarnation, g.group_unique_global_id FROM models m LEFT JOIN models_group g ON g.group_id = m.model_group_id AND g.group_user_id = m.model_user_id WHERE m.model_user_id = ",
    );
    select.push_bind(user.id);
    select.push(" AND m.model_deleted IS NULL AND m.model_id IN ");
    push_in(&mut select, &model_ids);
    let models: Vec<ModelAssignmentRow> =
        select.build_query_as().fetch_all(&mut *transaction).await?;
    let mut touched_group_ids: Vec<i64> = models
        .iter()
        .filter_map(|(_, group, _, _, _)| *group)
        .unique()
        .collect();

    let target_global = if let Some(group_id) = group_id {
        crate::ensure_owned(&mut *transaction, &GROUPS, &[group_id], user).await?;
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM models_group WHERE group_id = ? AND group_user_id = ? AND group_deleted_at IS NULL)",
        )
        .bind(group_id)
        .bind(user.id)
        .fetch_one(&mut *transaction)
        .await?;
        if !active {
            return Err(DbError::RowNotFound);
        }
        touched_group_ids.push(group_id);
        Some(sqlx::query_scalar::<_, String>("SELECT group_unique_global_id FROM models_group WHERE group_id = ? AND group_user_id = ?")
            .bind(group_id).bind(user.id).fetch_one(&mut *transaction).await?)
    } else {
        None
    };

    let timestamp = timestamp_or_now(update_timestamp);
    for (model_id, old_id, model, incarnation, old_global) in &models {
        if old_id.is_some() != old_global.is_some() || model.is_empty() || incarnation.is_empty() {
            return Err(DbError::Protocol(
                "Invalid model or group sync identity".into(),
            ));
        }
        if crate::causal_group_lifecycle::try_move(&mut transaction, user, *model_id, group_id)
            .await?
        {
            continue;
        }
        if crate::causal_model_move::try_move(
            &mut transaction,
            user,
            *model_id,
            group_id,
            &timestamp,
        )
        .await?
        {
            continue;
        }
        if *old_id == group_id {
            continue;
        }
        ensure_unambiguous_move(
            &mut transaction,
            user,
            model,
            old_global.as_deref(),
            target_global.as_deref(),
        )
        .await?;
        let event = crate::group_history::record_membership(
            &mut transaction,
            user,
            model,
            incarnation,
            &crate::group_history::MembershipAction::Move {
                from_group_global_id: old_global.clone(),
                to_group_global_id: target_global.clone(),
            },
        )
        .await?;
        if let Some(target) = &target_global {
            crate::group_history::record_group_member_added(
                &mut transaction,
                user,
                target,
                model,
                incarnation,
                &event,
            )
            .await?;
        }
    }

    let model_ids: Vec<i64> = models.iter().map(|(id, _, _, _, _)| *id).collect();
    let mut query_builder = QueryBuilder::new("UPDATE models SET model_group_id = ");
    query_builder.push_bind(group_id);
    query_builder.push(" WHERE model_id IN ");
    push_in(&mut query_builder, &model_ids);
    query_builder.push(" AND model_user_id = ");
    query_builder.push_bind(user.id);
    if let Some(group_id) = group_id {
        query_builder.push(" AND EXISTS (SELECT 1 FROM models_group WHERE group_id = ");
        query_builder.push_bind(group_id);
        query_builder.push(" AND group_user_id = ");
        query_builder.push_bind(user.id);
        query_builder.push(" AND group_deleted_at IS NULL)");
    }
    let result = query_builder.build().execute(&mut *transaction).await?;
    if group_id.is_some() && !model_ids.is_empty() && result.rows_affected() == 0 {
        return Err(DbError::RowNotFound);
    }

    set_timestamp_column(
        &mut *transaction,
        &GROUPS,
        "group_last_modified",
        &touched_group_ids,
        user,
        &timestamp,
    )
    .await?;

    transaction.commit().await
}

pub async fn add_empty_group(
    db: &DbContext,
    user: &User,
    group_name: &str,
    update_timestamp: Option<&str>,
) -> Result<ModelGroupMeta, DbError> {
    let now = time_now();
    let last_modified = update_timestamp.unwrap_or(&now).to_string();
    let unique_global_id = random_hex_32();
    let birth_id = random_hex_32();
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;

    let result = sqlx::query(
        "INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id, group_birth_id) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(group_name)
    .bind(&now)
    .bind(user.id)
    .bind(&last_modified)
    .bind(&unique_global_id)
    .bind(&birth_id)
    .execute(&mut *transaction)
    .await?;
    crate::causal_envelope::record_local_creation(
        &mut transaction,
        user,
        crate::causal_envelope::Entity {
            kind: crate::causal_envelope::EntityKind::Group,
            global_id: unique_global_id.clone(),
            birth_id: Some(birth_id),
        },
        crate::causal_envelope::Action::GroupCreated {
            name: group_name.to_owned(),
        },
    )
    .await?;
    transaction.commit().await?;

    Ok(ModelGroupMeta {
        id: result.last_insert_rowid(),
        name: group_name.to_string(),
        created: now,
        last_modified,
        resource_id: None,
        unique_global_id,
    })
}

/// Fails with [`DbError::RowNotFound`] when the user owns no group with this id.
pub async fn edit_group(
    db: &DbContext,
    user: &User,
    group_id: i64,
    group_name: &str,
    update_timestamp: Option<&str>,
    global_id: Option<&str>,
) -> Result<(), DbError> {
    crate::util::validate_global_id(global_id)?;
    let timestamp = timestamp_or_now(update_timestamp);
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    let existing: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT group_name, group_unique_global_id, group_birth_id FROM models_group WHERE group_id = ? AND group_user_id = ? AND group_deleted_at IS NULL",
    )
    .bind(group_id)
    .bind(user.id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some((old_name, old_global_id, birth_id)) = existing else {
        return Err(DbError::RowNotFound);
    };

    let target_global_id = global_id.unwrap_or(&old_global_id);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ? AND group_id != ?")
        .bind(user.id).bind(target_global_id).bind(group_id).fetch_one(&mut *transaction).await?;
    if target_global_id.is_empty() || count != 0 {
        return Err(DbError::Protocol(
            "Invalid or ambiguous group sync identity".into(),
        ));
    }
    if target_global_id != old_global_id {
        let conflicting_birth: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entity_births WHERE user_id = ? AND entity_kind = 'group' AND global_id = ? AND birth_id IS NOT ?)")
            .bind(user.id).bind(target_global_id).bind(&birth_id).fetch_one(&mut *transaction).await?;
        let history_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_causal_events WHERE user_id = ? AND group_global_id IN (?, ?))")
            .bind(user.id).bind(&old_global_id).bind(target_global_id).fetch_one(&mut *transaction).await?;
        if conflicting_birth || history_exists {
            return Err(DbError::Protocol(
                "Cannot change a group identity with causal history".into(),
            ));
        }
    }

    if crate::causal_group_lifecycle::try_edit(
        &mut transaction,
        user,
        group_id,
        group_name,
        &timestamp,
        target_global_id != old_global_id,
    )
    .await?
    {
        return transaction.commit().await;
    }
    if crate::causal_group_rename::try_edit(
        &mut transaction,
        user,
        group_id,
        group_name,
        &timestamp,
        target_global_id != old_global_id,
    )
    .await?
    {
        return transaction.commit().await;
    }

    let result = sqlx::query(
        "UPDATE models_group SET group_name = ?, group_last_modified = ?, group_unique_global_id = COALESCE(?, group_unique_global_id) WHERE group_id = ? AND group_user_id = ? AND group_deleted_at IS NULL",
    )
    .bind(group_name)
    .bind(timestamp)
    .bind(global_id)
    .bind(group_id)
    .bind(user.id)
    .execute(&mut *transaction)
    .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::RowNotFound);
    }
    if old_name != group_name {
        // Rekeying does not establish ancestry; sync must resolve it before enabling causal replication.
        crate::group_history::record_rename(
            &mut transaction,
            user,
            global_id.unwrap_or(&old_global_id),
            group_name,
        )
        .await?;
    }

    transaction.commit().await
}

pub async fn delete_group(db: &DbContext, user: &User, group_id: i64) -> Result<(), DbError> {
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    let group: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT group_unique_global_id, group_deleted_at FROM models_group WHERE group_id = ? AND group_user_id = ?",
    )
    .bind(group_id)
    .bind(user.id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some((global_id, None)) = group else {
        return Ok(());
    };
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?",
    )
    .bind(user.id)
    .bind(&global_id)
    .fetch_one(&mut *transaction)
    .await?;
    let invalid_members: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models m WHERE m.model_user_id = ? AND m.model_group_id = ? AND (m.model_unique_global_id = '' OR m.model_incarnation = '' OR (SELECT COUNT(*) FROM models other WHERE other.model_user_id = m.model_user_id AND other.model_unique_global_id = m.model_unique_global_id) != 1))")
        .bind(user.id).bind(group_id).fetch_one(&mut *transaction).await?;
    if global_id.is_empty() || count != 1 || invalid_members {
        return Err(DbError::Protocol(
            "Invalid or ambiguous sync identity".into(),
        ));
    }
    if crate::causal_group_lifecycle::try_delete(&mut transaction, user, group_id).await? {
        return transaction.commit().await;
    }
    crate::causal_model_move::pause_group_members(&mut transaction, user, group_id).await?;
    let operation_id = random_hex_32();
    let deletion_event = crate::group_history::record_lifecycle(
        &mut transaction,
        user,
        &global_id,
        &operation_id,
        true,
        None,
    )
    .await?;
    let members: Vec<(String, String)> = sqlx::query_as(
        "SELECT model_unique_global_id, model_incarnation FROM models WHERE model_group_id = ? AND model_user_id = ?",
    ).bind(group_id).bind(user.id).fetch_all(&mut *transaction).await?;
    for (model, incarnation) in members {
        crate::group_history::record_membership(
            &mut transaction,
            user,
            &model,
            &incarnation,
            &crate::group_history::MembershipAction::Detach {
                group_global_id: global_id.clone(),
                deletion_operation_id: operation_id.clone(),
                deletion_event: deletion_event.clone(),
            },
        )
        .await?;
    }
    sqlx::query("DELETE FROM group_recovery_members WHERE group_id = ?")
        .bind(group_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO group_recovery_members (group_id, model_id) SELECT ?, model_id FROM models WHERE model_group_id = ? AND model_user_id = ?",
    )
    .bind(group_id)
    .bind(group_id)
    .bind(user.id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE models SET model_group_id = NULL WHERE model_group_id = ? AND model_user_id = ?",
    )
    .bind(group_id)
    .bind(user.id)
    .execute(&mut *transaction)
    .await?;
    let deleted_at = time_now();
    sqlx::query(
        "UPDATE models_group SET group_deleted_at = ?, group_deletion_operation_id = ? WHERE group_id = ? AND group_user_id = ?",
    )
    .bind(&deleted_at)
    .bind(&operation_id)
    .bind(group_id)
    .bind(user.id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("INSERT INTO group_deletion_records (user_id, group_global_id, operation_id, deleted_at, group_revision, group_birth_id) SELECT ?, ?, ?, ?, group_revision, group_birth_id FROM models_group WHERE group_id = ? AND group_user_id = ?")
        .bind(user.id)
        .bind(global_id)
        .bind(&operation_id)
        .bind(deleted_at)
        .bind(group_id)
        .bind(user.id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE group_sync_heads SET last_deletion_operation_id = ? WHERE user_id = ? AND group_global_id = (SELECT group_unique_global_id FROM models_group WHERE group_id = ? AND group_user_id = ?)")
        .bind(operation_id).bind(user.id).bind(group_id).bind(user.id).execute(&mut *transaction).await?;
    transaction.commit().await?;

    Ok(())
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct DeletedGroup {
    pub id: i64,
    pub name: String,
    pub deleted: String,
    pub group_global_id: String,
    pub deletion_operation_id: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GroupRestoreRequest {
    pub id: i64,
    pub group_global_id: String,
    pub deletion_operation_id: String,
}

type GroupRestoreRow = (
    String,
    Option<String>,
    bool,
    Option<String>,
    Option<String>,
    Option<String>,
);

#[derive(serde::Serialize)]
pub struct GroupSyncSnapshot {
    pub groups: Vec<GroupSyncGroup>,
    pub assignments: Vec<GroupSyncAssignment>,
    pub deletions: Vec<GroupSyncDeletion>,
    pub heads: Vec<GroupSyncHead>,
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct GroupSyncHead {
    pub group_global_id: String,
    pub generation: i64,
    pub head: i64,
    pub state: String,
    pub last_deletion_operation_id: Option<String>,
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct GroupSyncGroup {
    pub global_id: String,
    pub birth_id: Option<String>,
    pub name: String,
    pub revision: i64,
    pub deleted_at: Option<String>,
    pub resource_global_id: Option<String>,
    #[serde(skip)]
    invalid_resource: bool,
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct GroupSyncAssignment {
    pub model_global_id: String,
    pub model_birth_id: Option<String>,
    pub incarnation: String,
    pub group_global_id: Option<String>,
    pub revision: i64,
    pub lifecycle_revision: i64,
    pub deleted_at: Option<String>,
    #[serde(skip)]
    invalid_group: bool,
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct GroupSyncDeletion {
    pub group_global_id: String,
    pub operation_id: String,
    pub deleted_at: String,
    pub restored_at: Option<String>,
    pub revision: i64,
    pub group_birth_id: Option<String>,
}

pub async fn get_group_sync_snapshot(
    db: &DbContext,
    user: &User,
) -> Result<GroupSyncSnapshot, DbError> {
    let mut transaction = db.begin().await?;
    let snapshot = get_group_sync_snapshot_in(&mut transaction, user).await?;
    transaction.commit().await?;
    Ok(snapshot)
}

pub(crate) async fn get_group_sync_snapshot_in(
    db: &mut SqliteConnection,
    user: &User,
) -> Result<GroupSyncSnapshot, DbError> {
    let groups: Vec<GroupSyncGroup> = sqlx::query_as(
        "SELECT g.group_unique_global_id AS global_id, g.group_birth_id AS birth_id, g.group_name AS name, g.group_revision AS revision, g.group_deleted_at AS deleted_at, r.resource_unique_global_id AS resource_global_id, (g.group_resource_id IS NOT NULL AND r.resource_id IS NULL) AS invalid_resource FROM models_group g LEFT JOIN resources r ON r.resource_id = g.group_resource_id AND r.resource_user_id = g.group_user_id WHERE g.group_user_id = ? ORDER BY g.group_id",
    )
    .bind(user.id)
    .fetch_all(&mut *db)
    .await?;
    let assignments: Vec<GroupSyncAssignment> = sqlx::query_as(
        "SELECT m.model_unique_global_id AS model_global_id, m.model_birth_id AS model_birth_id, m.model_incarnation AS incarnation, g.group_unique_global_id AS group_global_id, m.model_assignment_revision AS revision, m.model_lifecycle_revision AS lifecycle_revision, m.model_deleted AS deleted_at, (m.model_group_id IS NOT NULL AND g.group_id IS NULL) AS invalid_group FROM models m LEFT JOIN models_group g ON g.group_id = m.model_group_id AND g.group_user_id = m.model_user_id WHERE m.model_user_id = ? ORDER BY m.model_id",
    )
    .bind(user.id)
    .fetch_all(&mut *db)
    .await?;
    let deletions: Vec<GroupSyncDeletion> = sqlx::query_as(
        "SELECT group_global_id, operation_id, deleted_at, restored_at, group_revision AS revision, group_birth_id FROM group_deletion_records WHERE user_id = ? ORDER BY deleted_at, operation_id",
    )
    .bind(user.id)
    .fetch_all(&mut *db)
    .await?;
    let heads: Vec<GroupSyncHead> = sqlx::query_as(
        "SELECT group_global_id, generation, head, state, last_deletion_operation_id FROM group_sync_heads WHERE user_id = ? ORDER BY group_global_id",
    )
    .bind(user.id)
    .fetch_all(&mut *db)
    .await?;

    let mut group_ids = HashSet::new();
    let mut model_ids = HashSet::new();
    let mut incarnations = HashSet::new();
    if groups.iter().any(|group| {
        group.invalid_resource
            || group.global_id.is_empty()
            || !group_ids.insert(&group.global_id)
            || group
                .resource_global_id
                .as_ref()
                .is_some_and(String::is_empty)
    }) || assignments.iter().any(|model| {
        model.invalid_group
            || model.model_global_id.is_empty()
            || model.incarnation.is_empty()
            || !incarnations.insert(&model.incarnation)
            || !model_ids.insert(&model.model_global_id)
    }) || deletions
        .iter()
        .any(|record| record.group_global_id.is_empty() || record.operation_id.is_empty())
    {
        return Err(DbError::Protocol(
            "Invalid or ambiguous group sync identity".into(),
        ));
    }

    Ok(GroupSyncSnapshot {
        groups,
        assignments,
        deletions,
        heads,
    })
}

pub async fn get_trashed_groups(
    db: &DbContext,
    user: &User,
    retention_days: u32,
) -> Result<Vec<DeletedGroup>, DbError> {
    sqlx::query_as::<_, DeletedGroup>(
        "SELECT g.group_id AS id, g.group_name AS name, g.group_deleted_at AS deleted, g.group_unique_global_id AS group_global_id, CASE WHEN (SELECT COUNT(*) FROM models_group other WHERE other.group_user_id = g.group_user_id AND other.group_unique_global_id = g.group_unique_global_id) = 1 AND EXISTS (SELECT 1 FROM group_deletion_records r WHERE r.user_id = g.group_user_id AND r.group_global_id = g.group_unique_global_id AND r.operation_id = g.group_deletion_operation_id AND r.restored_at IS NULL) THEN g.group_deletion_operation_id ELSE NULL END AS deletion_operation_id FROM models_group g WHERE g.group_user_id = ? AND g.group_deleted_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-' || ? || ' days') ORDER BY g.group_deleted_at DESC, g.group_id DESC",
    )
    .bind(user.id)
    .bind(retention_days)
    .fetch_all(db)
    .await
}

#[allow(clippy::too_many_lines)] // Validate and restore the whole requested batch atomically.
pub async fn restore_groups(
    db: &DbContext,
    user: &User,
    groups: &[GroupRestoreRequest],
    retention_days: u32,
) -> Result<(), DbError> {
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    if groups.iter().map(|group| group.id).unique().count() != groups.len() {
        return Err(DbError::RowNotFound);
    }
    for group in groups.iter().unique_by(|group| group.id) {
        let group_id = group.id;
        let row: Option<GroupRestoreRow> = sqlx::query_as(
            "SELECT g.group_unique_global_id, g.group_deleted_at, COALESCE(g.group_deleted_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-' || ? || ' days'), 1), g.group_deletion_operation_id, r.restored_at, r.operation_id FROM models_group g LEFT JOIN group_deletion_records r ON r.user_id = g.group_user_id AND r.group_global_id = g.group_unique_global_id AND r.operation_id = g.group_deletion_operation_id WHERE g.group_id = ? AND g.group_user_id = ?",
        )
        .bind(retention_days)
        .bind(group_id)
        .bind(user.id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((global_id, deleted_at, within_retention, operation_id, restored_at, record_id)) =
            row
        else {
            return Err(DbError::RowNotFound);
        };
        if global_id == group.group_global_id
            && deleted_at.is_none()
            && operation_id.is_none()
            && crate::causal_group_lifecycle::restore_retry(
                &mut transaction,
                user,
                group_id,
                &group.deletion_operation_id,
            )
            .await?
        {
            continue;
        }
        if global_id != group.group_global_id
            || operation_id.as_deref() != Some(&group.deletion_operation_id)
            || record_id.as_deref() != Some(&group.deletion_operation_id)
        {
            return Err(DbError::RowNotFound);
        }
        let identity_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models_group WHERE group_user_id = ? AND group_unique_global_id = ?")
            .bind(user.id).bind(&global_id).fetch_one(&mut *transaction).await?;
        if identity_count != 1 {
            return Err(DbError::RowNotFound);
        }
        if deleted_at.is_none() {
            if restored_at.is_none() {
                return Err(DbError::RowNotFound);
            }
            continue;
        }
        if !within_retention || restored_at.is_some() {
            return Err(DbError::RowNotFound);
        }
        if crate::causal_group_lifecycle::try_restore(
            &mut transaction,
            user,
            group_id,
            &group.deletion_operation_id,
        )
        .await?
        {
            continue;
        }
        let deletion_event = crate::group_history::find_deletion_event(
            &mut transaction,
            user,
            &global_id,
            &group.deletion_operation_id,
        )
        .await?;
        crate::group_history::record_lifecycle(
            &mut transaction,
            user,
            &global_id,
            &group.deletion_operation_id,
            false,
            deletion_event.as_ref(),
        )
        .await?;
        let members: Vec<(String, String)> = sqlx::query_as(
            "SELECT model_unique_global_id, model_incarnation FROM models WHERE model_user_id = ? AND model_group_id IS NULL AND model_id IN (SELECT model_id FROM group_recovery_members WHERE group_id = ?)",
        ).bind(user.id).bind(group_id).fetch_all(&mut *transaction).await?;
        for (model, incarnation) in members {
            crate::group_history::record_membership(
                &mut transaction,
                user,
                &model,
                &incarnation,
                &crate::group_history::MembershipAction::Restore {
                    group_global_id: global_id.clone(),
                    deletion_operation_id: group.deletion_operation_id.clone(),
                    deletion_event: deletion_event.clone(),
                },
            )
            .await?;
        }
        sqlx::query("UPDATE models SET model_group_id = ? WHERE model_user_id = ? AND model_group_id IS NULL AND model_id IN (SELECT model_id FROM group_recovery_members WHERE group_id = ?)")
            .bind(group_id)
            .bind(user.id)
            .bind(group_id)
            .execute(&mut *transaction)
            .await?;
        let restored_at = time_now();
        sqlx::query("UPDATE models_group SET group_deleted_at = NULL, group_last_modified = ? WHERE group_id = ? AND group_user_id = ?")
            .bind(&restored_at)
            .bind(group_id)
            .bind(user.id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE group_deletion_records SET restored_at = ? WHERE user_id = ? AND group_global_id = ? AND operation_id = ? AND restored_at IS NULL")
            .bind(restored_at)
            .bind(user.id)
            .bind(global_id)
            .bind(&group.deletion_operation_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM group_recovery_members WHERE group_id = ?")
            .bind(group_id)
            .execute(&mut *transaction)
            .await?;
    }

    transaction.commit().await
}

pub async fn purge_trashed_groups(db: &DbContext, retention_days: u32) -> Result<u64, DbError> {
    crate::causal_group_lifecycle::purge_expired(db, retention_days).await
}

pub async fn delete_dead_groups(db: &DbContext) -> Result<(), DbError> {
    // No user filter needed: a group without models is dead regardless of owner. Trashed models
    // still count, so restoring one brings its group back.
    sqlx::query!(
        "DELETE FROM models_group
         WHERE group_id NOT IN (SELECT DISTINCT model_group_id FROM models WHERE model_group_id IS NOT NULL)"
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn get_group_count(
    db: &DbContext,
    user: &User,
    include_ungrouped_models: bool,
) -> Result<usize, DbError> {
    let base: usize = sqlx::query!(
        "SELECT COUNT(DISTINCT model_group_id) as count FROM models WHERE model_user_id = ? AND model_deleted IS NULL",
        user.id
    )
    .fetch_one(db)
    .await?
    .count
    .try_into()
    .unwrap_or(0);

    let empty: usize = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM models_group WHERE group_deleted_at IS NULL AND group_user_id = ? AND group_id NOT IN (SELECT model_group_id FROM models WHERE model_group_id IS NOT NULL)",
    )
    .bind(user.id)
    .fetch_one(db)
    .await?
    .try_into()
    .unwrap_or(0);

    let ungrouped: usize = if include_ungrouped_models {
        sqlx::query!(
            "SELECT COUNT(*) as count FROM models WHERE model_user_id = ? AND model_group_id IS NULL AND model_deleted IS NULL",
            user.id
        )
        .fetch_one(db)
        .await?
        .count
        .try_into()
        .unwrap_or(0)
    } else {
        0
    };

    Ok(base + empty + ungrouped)
}

pub struct GroupModelCount {
    pub id: i64,
    pub name: String,
    pub model_count: i64,
}

/// The user's groups holding at least one live model, with how many, ordered by name.
pub async fn get_group_model_counts(
    db: &DbContext,
    user: &User,
) -> Result<Vec<GroupModelCount>, DbError> {
    sqlx::query_as!(
        GroupModelCount,
        r#"SELECT group_id AS "id!", group_name AS "name!", COUNT(*) AS "model_count!: i64"
         FROM models_group
         INNER JOIN models ON models.model_group_id = models_group.group_id
         WHERE group_user_id = ? AND model_deleted IS NULL
         GROUP BY group_id
         ORDER BY group_name"#,
        user.id
    )
    .fetch_all(db)
    .await
}

pub async fn set_last_updated_on_groups(
    db: &DbContext,
    user: &User,
    group_ids: &[i64],
    timestamp: &str,
) -> Result<(), DbError> {
    set_timestamp_column(
        db,
        &GROUPS,
        "group_last_modified",
        group_ids,
        user,
        timestamp,
    )
    .await
}

#[cfg(test)]
mod tests {
    fn group(id: i64, name: &str) -> super::ModelGroup {
        super::ModelGroup::from_meta(super::ModelGroupMeta {
            id,
            name: name.into(),
            created: String::new(),
            last_modified: String::new(),
            resource_id: None,
            unique_global_id: String::new(),
        })
    }

    #[test]
    fn populated_group_wins_when_empty_and_populated_reads_overlap() {
        let mut populated = group(1, "populated");
        populated.models.push(super::Model {
            id: 11,
            name: "part".into(),
            blob: crate::model::blob::Blob {
                id: 21,
                sha256: "hash".into(),
                filetype: "stl".into(),
                size: 1,
                added: String::new(),
                disk_path: None,
            },
            link: None,
            description: None,
            added: String::new(),
            last_modified: String::new(),
            group: None,
            labels: Vec::new(),
            flags: super::ModelFlags::empty(),
            unique_global_id: String::new(),
            deleted: None,
        });
        let mut groups = vec![populated];

        super::append_empty_groups(
            &mut groups,
            vec![group(1, "stale empty"), group(2, "empty")],
        );

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].meta.name, "populated");
        assert_eq!(groups[0].models[0].id, 11);
        assert_eq!(groups[1].meta.id, 2);
        assert!(groups[1].models.is_empty());
    }
}
