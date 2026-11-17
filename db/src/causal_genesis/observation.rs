//! In-process snapshots establish a new observed boundary, never past ancestry.
use std::fmt::Write;

use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;

use super::{
    GenesisBundle, encode, invalid,
    materialization::{self, GroupSnapshot, Materialization},
    storage,
};
use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, Dot, Entity, EntityKind},
    db_context::DbContext,
    model::user::User,
    sync_peer::validate_local,
    user_db::SyncIdentity,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub(super) struct Snapshot {
    pub global_id: String,
    pub birth_id: Option<String>,
    pub generation: i64,
    pub head: i64,
    pub revision: i64,
    pub epoch: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<i64>,
    pub name: String,
    pub created: String,
    pub last_modified: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    observation_id: String,
    source_instance_id: String,
    source_account_id: String,
    source_group_id: i64,
    snapshot: Snapshot,
    materialization_digest: String,
    history: String,
}

/// Opaque local capture. Neither a wire credential nor proof of earlier history.
#[derive(Debug)]
pub struct GroupObservation {
    pub(super) owner: i64,
    pub(super) identity: SyncIdentity,
    pub(super) group_id: i64,
    pub(super) snapshot: Snapshot,
    pub(super) observation_id: String,
}

pub(super) fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");

            output
        })
}

pub(super) async fn snapshot(
    db: &mut SqliteConnection,
    user: &User,
    id: i64,
    populated: bool,
) -> Result<Snapshot, DbError> {
    let row: Snapshot = sqlx::query_as("SELECT g.group_unique_global_id AS global_id,g.group_birth_id AS birth_id,h.generation,h.head,g.group_revision AS revision,v.revision AS epoch,g.group_resource_id AS resource_id,g.group_name AS name,g.group_created AS created,g.group_last_modified AS last_modified FROM models_group g JOIN group_sync_heads h ON h.user_id=g.group_user_id AND h.group_global_id=g.group_unique_global_id JOIN group_observation_versions v ON v.group_id=g.group_id WHERE g.group_id=? AND g.group_user_id=? AND g.group_deleted_at IS NULL AND g.group_deletion_operation_id IS NULL AND (? OR g.group_resource_id IS NULL) AND h.state='active' AND h.identity_birth_id IS g.group_birth_id AND (? OR NOT EXISTS(SELECT 1 FROM models WHERE model_group_id=g.group_id)) AND NOT EXISTS(SELECT 1 FROM group_deletion_records WHERE user_id=g.group_user_id AND group_global_id=g.group_unique_global_id) AND NOT EXISTS(SELECT 1 FROM causal_genesis_bindings WHERE user_id=g.group_user_id AND entity_kind='group' AND global_id=g.group_unique_global_id) AND (SELECT COUNT(*) FROM models_group other WHERE other.group_user_id=g.group_user_id AND other.group_unique_global_id=g.group_unique_global_id)=1 AND length(CAST(g.group_name AS BLOB))+length(CAST(g.group_created AS BLOB))+length(CAST(g.group_last_modified AS BLOB))<=?")
        .bind(id).bind(user.id).bind(populated).bind(populated).bind(super::MAX_MATERIALIZATION_BYTES).fetch_optional(&mut *db).await?.ok_or_else(invalid)?;
    if row.name.len() > causal_envelope::MAX_NAME_BYTES || row.generation != 1 {
        return Err(invalid());
    }
    // Existing remote claims cannot be silently adopted by matching a global id.
    let remote: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM causal_events e JOIN causal_event_receipts r USING(user_id,writer_id,counter) WHERE e.user_id=? AND e.encoding='portable_v2' AND json_extract(e.envelope,'$.entity.kind')='group' AND json_extract(e.envelope,'$.entity.global_id')=?) OR EXISTS(SELECT 1 FROM received_group_renames WHERE user_id=? AND group_global_id=?)")
        .bind(user.id).bind(&row.global_id).bind(user.id).bind(&row.global_id).fetch_one(&mut *db).await?;
    if remote {
        return Err(invalid());
    }
    if row.birth_id.is_some() {
        super::checks::uncontested(
            db,
            user,
            &Entity {
                kind: EntityKind::Group,
                global_id: row.global_id.clone(),
                birth_id: row.birth_id.clone(),
            },
        )
        .await?;
    } else {
        let registered: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entity_births WHERE user_id=? AND entity_kind='group' AND global_id=?)")
            .bind(user.id).bind(&row.global_id).fetch_one(db).await?;
        if registered {
            return Err(invalid());
        }
    }

    Ok(row)
}

pub async fn observe(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    group_id: i64,
) -> Result<GroupObservation, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut tx, user, identity).await?;
    let snapshot = snapshot(&mut tx, user, group_id, false).await?;
    tx.commit().await?;

    Ok(GroupObservation {
        owner: user.id,
        identity: identity.clone(),
        group_id,
        snapshot,
        observation_id: crate::random_hex_32(),
    })
}

pub(super) fn evidence(
    observation: &GroupObservation,
) -> Result<(String, Materialization, String), DbError> {
    let materialization = Materialization::Group(GroupSnapshot {
        name: observation.snapshot.name.clone(),
        created: observation.snapshot.created.clone(),
        last_modified: observation.snapshot.last_modified.clone(),
    });
    let json = encode(&materialization)?;
    let proof = Evidence {
        observation_id: observation.observation_id.clone(),
        source_instance_id: observation.identity.instance_id.clone(),
        source_account_id: observation.identity.account_id.clone(),
        source_group_id: observation.group_id,
        snapshot: observation.snapshot.clone(),
        materialization_digest: digest(&json),
        history: "unknown_before_observation".into(),
    };

    Ok((encode(&proof)?, materialization, json))
}

/// Validates immutable observed root metadata, without declaring older history complete.
pub fn validate(
    root: &CausalEnvelope,
    materialization: &str,
    proof: &str,
    source: &SyncIdentity,
) -> Result<String, DbError> {
    let evidence: Evidence = serde_json::from_str(proof).map_err(|_| invalid())?;
    let snapshot: GroupSnapshot = serde_json::from_str(materialization).map_err(|_| invalid())?;
    if !matches!(&root.action,Action::LegacyObserved{observation_digest} if *observation_digest==digest(proof))
        || root.entity.kind != EntityKind::Group
        || root.entity.birth_id.is_none()
        || !root.parents.is_empty()
        || !root.dependencies.is_empty()
        || evidence.history != "unknown_before_observation"
        || evidence.source_instance_id != source.instance_id
        || evidence.source_account_id != source.account_id
        || evidence.snapshot.global_id != root.entity.global_id
        || evidence
            .snapshot
            .birth_id
            .as_ref()
            .is_some_and(|birth| Some(birth) != root.entity.birth_id.as_ref())
        || evidence.materialization_digest != digest(materialization)
        || snapshot.name != evidence.snapshot.name
        || snapshot.created != evidence.snapshot.created
        || snapshot.last_modified != evidence.snapshot.last_modified
        || snapshot.name.len() > causal_envelope::MAX_NAME_BYTES
    {
        return Err(invalid());
    }

    Ok(snapshot.name)
}

async fn validate_retry(
    db: &mut SqliteConnection,
    user: &User,
    observation: &GroupObservation,
    entity: &Entity,
) -> Result<(), DbError> {
    let unchanged:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM causal_genesis_bindings b JOIN models_group g ON g.group_id=b.local_id AND g.group_user_id=b.user_id AND g.group_unique_global_id=b.global_id AND g.group_birth_id=b.birth_id JOIN group_observation_versions v ON v.group_id=g.group_id AND v.revision=b.local_observation_revision JOIN group_sync_heads h ON h.user_id=g.group_user_id AND h.group_global_id=g.group_unique_global_id WHERE b.user_id=? AND b.entity_kind='group' AND b.birth_id=? AND g.group_id=? AND g.group_name=? AND g.group_created=? AND g.group_last_modified=? AND g.group_deleted_at IS NULL AND g.group_deletion_operation_id IS NULL AND g.group_resource_id IS NULL AND h.state='active' AND h.identity_birth_id=g.group_birth_id AND h.generation=json_extract(b.local_guard,'$.generation') AND h.head=json_extract(b.local_guard,'$.head') AND g.group_revision=json_extract(b.local_guard,'$.revision') AND NOT EXISTS(SELECT 1 FROM models WHERE model_group_id=g.group_id) AND (SELECT COUNT(*) FROM models_group other WHERE other.group_user_id=g.group_user_id AND other.group_unique_global_id=g.group_unique_global_id)=1)")
        .bind(user.id).bind(&entity.birth_id).bind(observation.group_id).bind(&observation.snapshot.name).bind(&observation.snapshot.created).bind(&observation.snapshot.last_modified).fetch_one(&mut *db).await?;
    if !unchanged {
        return Err(invalid());
    }
    super::checks::uncontested(db, user, entity).await
}

pub async fn enroll(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    observation: &GroupObservation,
) -> Result<GenesisBundle, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut tx, user, identity).await?;
    if observation.owner != user.id || observation.identity != *identity {
        return Err(invalid());
    }
    let (proof, materialization, materialization_json) = evidence(observation)?;
    let prior:Option<(String,String)>=sqlx::query_as("SELECT e.envelope,b.observation_evidence FROM causal_genesis_bindings b JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.initialization_kind='observation' AND b.origin='local' AND json_extract(b.observation_evidence,'$.observation_id')=? AND b.local_instance_id=? AND b.local_account_id=?")
        .bind(user.id).bind(&observation.observation_id).bind(&identity.instance_id).bind(&identity.account_id).fetch_optional(&mut *tx).await?;
    if let Some((wire, stored)) = prior {
        if stored != proof {
            return Err(invalid());
        }
        let envelope = causal_envelope::parse_causal_envelope(&wire).map_err(|_| invalid())?;
        validate(&envelope, &materialization_json, &proof, identity)?;
        validate_retry(&mut tx, user, observation, &envelope.entity).await?;
        tx.commit().await?;

        return Ok(GenesisBundle {
            envelope,
            wire,
            source_identity: identity.clone(),
            materialization,
            materialization_json,
            capability: "group_lifecycle_v1".into(),
            observation_evidence: Some(proof),
        });
    }
    if snapshot(&mut tx, user, observation.group_id, false).await? != observation.snapshot {
        return Err(invalid());
    }
    let birth = observation
        .snapshot
        .birth_id
        .clone()
        .unwrap_or_else(crate::random_hex_32);
    if observation.snapshot.birth_id.is_none() {
        sqlx::query("INSERT INTO group_birth_adoption_permits(group_id,user_id,account_id,instance_id,global_id,birth_id,observation_revision,head) VALUES(?,?,?,?,?,?,?,?)")
            .bind(observation.group_id).bind(user.id).bind(&identity.account_id).bind(&identity.instance_id).bind(&observation.snapshot.global_id).bind(&birth).bind(observation.snapshot.epoch).bind(observation.snapshot.head).execute(&mut *tx).await?;
        sqlx::query("UPDATE models_group SET group_birth_id=? WHERE group_id=? AND group_user_id=? AND group_birth_id IS NULL")
            .bind(&birth).bind(observation.group_id).bind(user.id).execute(&mut *tx).await?;
    }
    let entity = Entity {
        kind: EntityKind::Group,
        global_id: observation.snapshot.global_id.clone(),
        birth_id: Some(birth),
    };
    let guard =
        materialization::current_guard(&mut tx, user, &entity, observation.group_id).await?;
    let (writer_id, counter) = crate::causal_clock::next(&mut tx, user.id).await?;
    let envelope = CausalEnvelope {
        version: 2,
        entity,
        event: Dot {
            writer_id,
            counter: counter.to_string(),
        },
        parents: vec![],
        dependencies: vec![],
        action: Action::LegacyObserved {
            observation_digest: digest(&proof),
        },
    };
    let wire = causal_envelope::canonical_causal_envelope(&envelope).map_err(|_| invalid())?;
    sqlx::query("INSERT INTO causal_events(user_id,writer_id,counter,envelope) VALUES(?,?,?,?)")
        .bind(user.id)
        .bind(&envelope.event.writer_id)
        .bind(counter)
        .bind(&wire)
        .execute(&mut *tx)
        .await?;
    let bundle = GenesisBundle {
        envelope,
        wire,
        source_identity: identity.clone(),
        materialization,
        materialization_json,
        capability: "group_lifecycle_v1".into(),
        observation_evidence: Some(proof),
    };
    storage::write_binding(
        &mut tx,
        user,
        identity,
        &bundle,
        observation.group_id,
        &guard,
        "local",
    )
    .await?;
    tx.commit().await?;

    Ok(bundle)
}
