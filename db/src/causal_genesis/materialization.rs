use sqlx::SqliteConnection;

use crate::{DbError, causal_envelope::Entity, model::user::User};

use super::invalid;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct BlobDescriptor {
    pub hash: String,
    pub filetype: String,
    pub size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub(super) struct GroupSnapshot {
    pub name: String,
    pub created: String,
    pub last_modified: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct ModelSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<String>,
    pub name: String,
    pub link: Option<String>,
    pub description: Option<String>,
    pub flags: i64,
    pub added: String,
    pub last_modified: String,
    pub blob: BlobDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Materialization {
    Group(GroupSnapshot),
    Model(ModelSnapshot),
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Guard {
    Group {
        generation: i64,
        head: i64,
        revision: i64,
    },
    Model {
        incarnation: String,
        blob_id: i64,
        assignment_revision: i64,
        lifecycle_revision: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        observation_revision: Option<i64>,
    },
}

#[derive(sqlx::FromRow)]
struct GroupRow {
    global_id: String,
    birth_id: Option<String>,
    generation: i64,
    name: String,
    created: String,
    last_modified: String,
}

#[derive(sqlx::FromRow)]
struct ModelRow {
    global_id: String,
    birth_id: Option<String>,
    incarnation: String,
    blob_id: i64,
    name: String,
    link: Option<String>,
    description: Option<String>,
    flags: i64,
    added: String,
    last_modified: String,
    hash: String,
    filetype: String,
    size: i64,
}

pub(super) async fn read_group(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
) -> Result<(Entity, Materialization, Guard), DbError> {
    let row: GroupRow = sqlx::query_as("SELECT g.group_unique_global_id AS global_id, g.group_birth_id AS birth_id, h.generation, g.group_name AS name, g.group_created AS created, g.group_last_modified AS last_modified FROM models_group g JOIN group_sync_heads h ON h.user_id = g.group_user_id AND h.group_global_id = g.group_unique_global_id WHERE g.group_user_id = ? AND g.group_id = ? AND g.group_revision = 0 AND g.group_deleted_at IS NULL AND g.group_deletion_operation_id IS NULL AND g.group_resource_id IS NULL AND h.generation = 1 AND h.head = 1 AND h.state = 'active' AND NOT EXISTS (SELECT 1 FROM models WHERE model_group_id = g.group_id) AND (SELECT COUNT(*) FROM models_group other WHERE other.group_user_id = g.group_user_id AND other.group_unique_global_id = g.group_unique_global_id) = 1 AND length(CAST(g.group_name AS BLOB)) + length(CAST(g.group_created AS BLOB)) + length(CAST(g.group_last_modified AS BLOB)) <= ?")
        .bind(user.id).bind(id).bind(super::MAX_MATERIALIZATION_BYTES).fetch_optional(db).await?.ok_or_else(invalid)?;

    Ok((
        Entity {
            kind: crate::causal_envelope::EntityKind::Group,
            global_id: row.global_id,
            birth_id: row.birth_id,
        },
        Materialization::Group(GroupSnapshot {
            name: row.name,
            created: row.created,
            last_modified: row.last_modified,
        }),
        Guard::Group {
            generation: row.generation,
            head: 1,
            revision: 0,
        },
    ))
}

pub(super) async fn read_model(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
) -> Result<(Entity, Materialization, Guard), DbError> {
    let row: ModelRow = sqlx::query_as("SELECT m.model_unique_global_id AS global_id, m.model_birth_id AS birth_id, m.model_incarnation AS incarnation, m.model_blob_id AS blob_id, m.model_name AS name, m.model_url AS link, m.model_desc AS description, m.model_flags AS flags, m.model_added AS added, m.model_last_modified AS last_modified, b.blob_sha256 AS hash, b.blob_filetype AS filetype, b.blob_size AS size FROM models m JOIN blobs b ON b.blob_id = m.model_blob_id WHERE m.model_user_id = ? AND m.model_id = ? AND m.model_genesis_unchanged = 1 AND m.model_assignment_revision = 0 AND m.model_lifecycle_revision = 0 AND m.model_group_id IS NULL AND m.model_deleted IS NULL AND m.model_incarnation != '' AND NOT EXISTS (SELECT 1 FROM models_labels WHERE model_id = m.model_id) AND (SELECT COUNT(*) FROM models other WHERE other.model_user_id = m.model_user_id AND other.model_unique_global_id = m.model_unique_global_id) = 1 AND length(CAST(m.model_name AS BLOB)) + COALESCE(length(CAST(m.model_url AS BLOB)), 0) + COALESCE(length(CAST(m.model_desc AS BLOB)), 0) + length(CAST(m.model_added AS BLOB)) + length(CAST(m.model_last_modified AS BLOB)) + length(CAST(b.blob_sha256 AS BLOB)) + length(CAST(b.blob_filetype AS BLOB)) <= ?")
        .bind(user.id).bind(id).bind(super::MAX_MATERIALIZATION_BYTES).fetch_optional(db).await?.ok_or_else(invalid)?;
    if !crate::is_blob_sha256(&row.hash) || row.size < 0 || row.filetype.is_empty() {
        return Err(invalid());
    }

    Ok((
        Entity {
            kind: crate::causal_envelope::EntityKind::Model,
            global_id: row.global_id,
            birth_id: row.birth_id,
        },
        Materialization::Model(ModelSnapshot {
            deleted: None,
            name: row.name,
            link: row.link,
            description: row.description,
            flags: row.flags,
            added: row.added,
            last_modified: row.last_modified,
            blob: BlobDescriptor {
                hash: row.hash,
                filetype: row.filetype,
                size: row.size,
            },
        }),
        Guard::Model {
            incarnation: row.incarnation,
            blob_id: row.blob_id,
            assignment_revision: 0,
            lifecycle_revision: 0,
            observation_revision: None,
        },
    ))
}

pub(super) async fn validate_blob(
    db: &mut SqliteConnection,
    value: &Materialization,
    local_blob_id: Option<i64>,
) -> Result<(), DbError> {
    match (value, local_blob_id) {
        (Materialization::Group(_), None) => {}
        (Materialization::Model(snapshot), Some(id)) => {
            // This checks stored descriptors only. File transfer must separately
            // verify actual bytes; source blob ids and filesystem paths never travel.
            let matches: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM blobs WHERE blob_id = ? AND blob_sha256 = ? AND blob_filetype = ? AND blob_size = ?)")
                .bind(id).bind(&snapshot.blob.hash).bind(&snapshot.blob.filetype).bind(snapshot.blob.size).fetch_one(db).await?;
            if !matches {
                return Err(invalid());
            }
        }
        _ => return Err(invalid()),
    }

    Ok(())
}

pub(super) async fn install(
    db: &mut SqliteConnection,
    user: &User,
    entity: &Entity,
    value: &Materialization,
    local_blob_id: Option<i64>,
) -> Result<(i64, Guard), DbError> {
    let id = match value {
        Materialization::Group(group) => {
            sqlx::query("INSERT INTO models_group (group_name, group_created, group_last_modified, group_user_id, group_unique_global_id, group_birth_id) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(&group.name).bind(&group.created).bind(&group.last_modified).bind(user.id).bind(&entity.global_id).bind(&entity.birth_id).execute(&mut *db).await?.last_insert_rowid()
        }
        Materialization::Model(model) => {
            let highwater: i64 = sqlx::query_scalar("SELECT highwater FROM object_id_highwater WHERE object_kind = 'model'")
                .fetch_one(&mut *db).await?;
            let id = highwater.checked_add(1).ok_or_else(invalid)?;
            sqlx::query("INSERT INTO models (model_id, model_name, model_blob_id, model_url, model_desc, model_flags, model_added, model_last_modified, model_user_id, model_unique_global_id, model_birth_id, model_deleted) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(id).bind(&model.name).bind(local_blob_id).bind(&model.link).bind(&model.description).bind(model.flags).bind(&model.added).bind(&model.last_modified).bind(user.id).bind(&entity.global_id).bind(&entity.birth_id).bind(&model.deleted).execute(&mut *db).await?;
            id
        }
    };
    let guard = current_guard(db, user, entity, id).await?;

    Ok((id, guard))
}

pub(super) async fn current_guard(
    db: &mut SqliteConnection,
    user: &User,
    entity: &Entity,
    id: i64,
) -> Result<Guard, DbError> {
    match entity.kind {
        crate::causal_envelope::EntityKind::Group => {
            let (generation, head, revision): (i64, i64, i64) = sqlx::query_as("SELECT h.generation, h.head, g.group_revision FROM models_group g JOIN group_sync_heads h ON h.user_id = g.group_user_id AND h.group_global_id = g.group_unique_global_id WHERE g.group_id = ? AND g.group_user_id = ? AND g.group_unique_global_id = ? AND g.group_birth_id = ?")
                .bind(id).bind(user.id).bind(&entity.global_id).bind(&entity.birth_id).fetch_optional(db).await?.ok_or_else(invalid)?;
            Ok(Guard::Group {
                generation,
                head,
                revision,
            })
        }
        crate::causal_envelope::EntityKind::Model => {
            let (incarnation, blob_id, assignment_revision, lifecycle_revision): (String, i64, i64, i64) = sqlx::query_as("SELECT model_incarnation, model_blob_id, model_assignment_revision, model_lifecycle_revision FROM models WHERE model_id = ? AND model_user_id = ? AND model_unique_global_id = ? AND model_birth_id = ?")
                .bind(id).bind(user.id).bind(&entity.global_id).bind(&entity.birth_id).fetch_optional(db).await?.ok_or_else(invalid)?;
            Ok(Guard::Model {
                incarnation,
                blob_id,
                assignment_revision,
                lifecycle_revision,
                observation_revision: None,
            })
        }
    }
}
