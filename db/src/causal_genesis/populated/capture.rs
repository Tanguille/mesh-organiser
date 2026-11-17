use super::super::{
    invalid,
    materialization::{BlobDescriptor, Materialization, ModelSnapshot},
    observation,
};
use crate::{
    DbError,
    causal_envelope::{Entity, EntityKind, MAX_CONTEXT_ITEMS},
    model::user::User,
};
use sqlx::SqliteConnection;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub(in crate::causal_genesis) struct Model {
    pub id: i64,
    pub deleted: Option<String>,
    pub global_id: String,
    pub birth_id: Option<String>,
    pub incarnation: String,
    pub epoch: i64,
    pub assignment_revision: i64,
    pub lifecycle_revision: i64,
    pub blob_id: i64,
    pub name: String,
    pub link: Option<String>,
    pub description: Option<String>,
    pub flags: i64,
    pub added: String,
    pub last_modified: String,
    pub hash: String,
    pub filetype: String,
    pub size: i64,
}
impl Model {
    pub fn materialization(&self) -> Materialization {
        Materialization::Model(ModelSnapshot {
            deleted: self.deleted.clone(),
            name: self.name.clone(),
            link: self.link.clone(),
            description: self.description.clone(),
            flags: self.flags,
            added: self.added.clone(),
            last_modified: self.last_modified.clone(),
            blob: BlobDescriptor {
                hash: self.hash.clone(),
                filetype: self.filetype.clone(),
                size: self.size,
            },
        })
    }
    pub fn entity(&self) -> Entity {
        Entity {
            kind: EntityKind::Model,
            global_id: self.global_id.clone(),
            birth_id: self.birth_id.clone(),
        }
    }
}

pub(super) async fn read(
    db: &mut SqliteConnection,
    user: &User,
    group_id: i64,
) -> Result<(observation::Snapshot, Vec<Model>), DbError> {
    let group = observation::snapshot(db, user, group_id, true).await?;
    let (count,bytes,bad):(i64,i64,bool)=sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(length(CAST(model_name AS BLOB))+COALESCE(length(CAST(model_url AS BLOB)),0)+COALESCE(length(CAST(model_desc AS BLOB)),0)+COALESCE(length(CAST(model_deleted AS BLOB)),0)),0),EXISTS(SELECT 1 FROM models m WHERE m.model_group_id=? AND (m.model_user_id!=?)) FROM models WHERE model_group_id=?")
        .bind(group_id).bind(user.id).bind(group_id).fetch_one(&mut *db).await?;
    if bad
        || usize::try_from(count).map_err(|_| invalid())? > MAX_CONTEXT_ITEMS
        || bytes > 16 * 1024 * 1024
    {
        return Err(invalid());
    }
    let models:Vec<Model>=sqlx::query_as("SELECT m.model_id AS id,m.model_deleted AS deleted,m.model_unique_global_id AS global_id,m.model_birth_id AS birth_id,m.model_incarnation AS incarnation,v.revision AS epoch,m.model_assignment_revision AS assignment_revision,m.model_lifecycle_revision AS lifecycle_revision,m.model_blob_id AS blob_id,m.model_name AS name,m.model_url AS link,m.model_desc AS description,m.model_flags AS flags,m.model_added AS added,m.model_last_modified AS last_modified,b.blob_sha256 AS hash,b.blob_filetype AS filetype,b.blob_size AS size FROM models m JOIN model_observation_versions v ON v.model_id=m.model_id JOIN blobs b ON b.blob_id=m.model_blob_id WHERE m.model_group_id=? AND m.model_user_id=? ORDER BY m.model_unique_global_id,m.model_id")
        .bind(group_id).bind(user.id).fetch_all(&mut *db).await?;
    if models.len() != usize::try_from(count).map_err(|_| invalid())? {
        return Err(invalid());
    }
    for model in &models {
        identity(db, user, model).await?;
    }
    Ok((group, models))
}

async fn identity(db: &mut SqliteConnection, user: &User, model: &Model) -> Result<(), DbError> {
    if !crate::is_blob_sha256(&model.hash)
        || model.filetype.is_empty()
        || model.size < 0
        || model.incarnation.is_empty()
    {
        return Err(invalid());
    }
    let ambiguous:bool=sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM models WHERE model_user_id=? AND model_unique_global_id=?)!=1 OR EXISTS(SELECT 1 FROM causal_genesis_bindings WHERE user_id=? AND entity_kind='model' AND (global_id=? OR birth_id=?)) OR EXISTS(SELECT 1 FROM entity_births WHERE user_id=? AND entity_kind='model' AND ((global_id=? AND birth_id IS NOT ?) OR (birth_id=? AND global_id!=?))) OR EXISTS(SELECT 1 FROM unknown_model_identity_tombstones WHERE user_id=? AND model_global_id=?) OR EXISTS(SELECT 1 FROM membership_causal_events WHERE user_id=? AND ((model_global_id=? AND (model_incarnation!=? OR (model_birth_id IS NOT NULL AND model_birth_id IS NOT ?))) OR (model_incarnation=? AND model_global_id!=?))) OR EXISTS(SELECT 1 FROM causal_events e JOIN causal_event_receipts r USING(user_id,writer_id,counter) WHERE e.user_id=? AND e.encoding='portable_v2' AND json_extract(e.envelope,'$.entity.kind')='model' AND (json_extract(e.envelope,'$.entity.global_id')=? OR json_extract(e.envelope,'$.entity.birth_id')=?)) OR EXISTS(SELECT 1 FROM local_creation_records c JOIN causal_events e USING(user_id,writer_id,counter) WHERE c.user_id=? AND c.entity_kind='model' AND c.birth_id=? AND json_extract(e.envelope,'$.entity.global_id')!=?)")
        .bind(user.id).bind(&model.global_id).bind(user.id).bind(&model.global_id).bind(&model.birth_id)
        .bind(user.id).bind(&model.global_id).bind(&model.birth_id).bind(&model.birth_id).bind(&model.global_id)
        .bind(user.id).bind(&model.global_id).bind(user.id).bind(&model.global_id).bind(&model.incarnation).bind(&model.birth_id).bind(&model.incarnation).bind(&model.global_id)
        .bind(user.id).bind(&model.global_id).bind(&model.birth_id).bind(user.id).bind(&model.birth_id).bind(&model.global_id).fetch_one(db).await?;
    if ambiguous {
        return Err(invalid());
    }
    Ok(())
}
