use indexmap::IndexMap;

use crate::{
    DbError, RESOURCES,
    db_context::DbContext,
    ensure_owned,
    group_db::{self, GroupFilterOptions, GroupOrderBy},
    model::{
        model_group::ModelGroup,
        resource::{ResourceFlags, ResourceMeta},
        user::User,
    },
    random_hex_32, set_timestamp_column, time_now, timestamp_or_now, validate_global_id,
};

pub async fn get_resources(db: &DbContext, user: &User) -> Result<Vec<ResourceMeta>, DbError> {
    sqlx::query_as("SELECT * FROM resources WHERE resource_user_id = ? ORDER BY resource_name ASC")
        .bind(user.id)
        .fetch_all(db)
        .await
}

pub async fn get_groups_for_resource(
    db: &DbContext,
    user: &User,
    resource_id: i64,
) -> Result<Vec<ModelGroup>, DbError> {
    let rows = sqlx::query!(
        "SELECT models_group.group_id FROM models_group WHERE models_group.group_resource_id = ? AND models_group.group_user_id = ?",
        resource_id,
        user.id
    )
    .fetch_all(db)
    .await?;

    let groups = group_db::get_groups(
        db,
        user,
        GroupFilterOptions {
            group_ids: Some(rows.iter().map(|r| r.group_id).collect()),
            order_by: Some(GroupOrderBy::NameAsc),
            ..Default::default()
        },
    )
    .await?;

    Ok(groups.items)
}

pub async fn get_group_id_to_resource_map(
    db: &DbContext,
    user: &User,
) -> Result<IndexMap<i64, ResourceMeta>, DbError> {
    let rows = sqlx::query(
        "SELECT models_group.group_id, resources.*
            FROM models_group
            INNER JOIN resources ON models_group.group_resource_id = resources.resource_id
            WHERE resources.resource_user_id = ?",
    )
    .bind(user.id)
    .fetch_all(db)
    .await?;

    rows.iter()
        .map(|row| {
            Ok((
                sqlx::Row::try_get(row, "group_id")?,
                sqlx::FromRow::from_row(row)?,
            ))
        })
        .collect()
}

pub async fn get_resource_meta_by_id(
    db: &DbContext,
    user: &User,
    id: i64,
) -> Result<Option<ResourceMeta>, DbError> {
    sqlx::query_as("SELECT * FROM resources WHERE resource_id = ? AND resource_user_id = ?")
        .bind(id)
        .bind(user.id)
        .fetch_optional(db)
        .await
}

pub async fn add_resource(
    db: &DbContext,
    user: &User,
    name: &str,
    update_timestamp: Option<&str>,
) -> Result<ResourceMeta, DbError> {
    let now = time_now();
    let unique_global_id = random_hex_32();
    let last_modified = update_timestamp.unwrap_or(&now).to_string();

    let result = sqlx::query!(
        "INSERT INTO resources (resource_name, resource_created, resource_user_id, resource_unique_global_id, resource_last_modified)
            VALUES (?, ?, ?, ?, ?)",
        name,
        now,
        user.id,
        unique_global_id,
        last_modified
    )
    .execute(db)
    .await?;

    // Flags match the schema's DEFAULT 0.
    Ok(ResourceMeta {
        id: result.last_insert_rowid(),
        name: name.to_string(),
        flags: ResourceFlags::empty(),
        created: now,
        last_modified,
        unique_global_id,
    })
}

pub async fn delete_resource(db: &DbContext, user: &User, resource_id: i64) -> Result<(), DbError> {
    sqlx::query!(
        "DELETE FROM resources WHERE resource_id = ? AND resource_user_id = ?",
        resource_id,
        user.id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn edit_resource(
    db: &DbContext,
    user: &User,
    resource_id: i64,
    name: &str,
    flags: ResourceFlags,
    update_timestamp: Option<&str>,
    global_id: Option<&str>,
) -> Result<(), DbError> {
    validate_global_id(global_id)?;
    let bits = i64::from(flags.bits());
    let timestamp = timestamp_or_now(update_timestamp);

    sqlx::query!(
        "UPDATE resources SET resource_name = ?, resource_flags = ?, resource_last_modified = ?, resource_unique_global_id = COALESCE(?, resource_unique_global_id) WHERE resource_id = ? AND resource_user_id = ?",
        name,
        bits,
        timestamp,
        global_id,
        resource_id,
        user.id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn set_last_updated_on_resource(
    db: &DbContext,
    user: &User,
    resource_id: i64,
    timestamp: &str,
) -> Result<(), DbError> {
    set_timestamp_column(
        db,
        &RESOURCES,
        "resource_last_modified",
        &[resource_id],
        user,
        timestamp,
    )
    .await
}

pub async fn set_resource_on_group(
    db: &DbContext,
    user: &User,
    resource_id: Option<i64>,
    group_id: i64,
    update_timestamp: Option<&str>,
) -> Result<(), DbError> {
    // Lightweight existence check; a group with zero models is treated as not found.
    let Some(group_row) = sqlx::query!(
        "SELECT group_resource_id FROM models_group
            WHERE group_id = ? AND group_user_id = ?
            AND EXISTS (SELECT 1 FROM models WHERE model_group_id = models_group.group_id AND model_user_id = ?)",
        group_id,
        user.id,
        user.id
    )
    .fetch_optional(db)
    .await?
    else {
        return Err(DbError::RowNotFound);
    };

    if let Some(resource_id) = resource_id {
        ensure_owned(db, &RESOURCES, &[resource_id], user).await?;
    }

    sqlx::query!(
        "UPDATE models_group SET group_resource_id = ? WHERE group_id = ? AND group_user_id = ?",
        resource_id,
        group_id,
        user.id
    )
    .execute(db)
    .await?;

    let timestamp = timestamp_or_now(update_timestamp);
    if let Some(resource_id) = resource_id {
        set_last_updated_on_resource(db, user, resource_id, &timestamp).await?;
    }

    if let Some(resource_id) = group_row.group_resource_id {
        set_last_updated_on_resource(db, user, resource_id, &timestamp).await?;
    }

    Ok(())
}
