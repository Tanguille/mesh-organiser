//! Atomic observed membership boundaries. No previous moves are manufactured.
mod capture;
mod identity;
pub(super) mod model_identity;
pub(super) use identity::enroll_identity;
mod labels;
mod persistence;
mod project;
pub mod transport;
use sqlx::SqliteConnection;

use super::{
    GenesisBundle, encode, invalid,
    materialization::{self, Guard, Materialization},
    observation,
};
use crate::{
    DbError,
    causal_envelope::{Action, CausalEnvelope, Dependency, Dot, Entity, EntityKind},
    db_context::DbContext,
    model::user::User,
    sync_peer::{PeerBinding, validate_local},
    user_db::SyncIdentity,
};

#[derive(Debug)]
pub struct PopulatedGroupObservation {
    owner: i64,
    identity: SyncIdentity,
    group_id: i64,
    batch_id: String,
    group: observation::Snapshot,
    models: Vec<capture::Model>,
    labels: labels::Snapshot,
    project: Option<project::Snapshot>,
}

#[derive(PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct IdentitySnapshot {
    #[serde(flatten)]
    pub group: observation::Snapshot,
    // Empty snapshots retain the original decision encoding and retry identity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) models: Vec<capture::Model>,
}
impl IdentitySnapshot {
    pub const fn has_models(&self) -> bool {
        !self.models.is_empty()
    }

    pub async fn read(
        db: &mut SqliteConnection,
        user: &User,
        group_id: i64,
    ) -> Result<Self, DbError> {
        let (group, models) = capture::read(db, user, group_id).await?;
        let labeled: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models_labels ml JOIN models m ON m.model_id=ml.model_id WHERE m.model_group_id=?)")
            .bind(group_id).fetch_one(db).await?;
        if group.resource_id.is_some() || labeled {
            return Err(invalid());
        }

        Ok(Self { group, models })
    }
}
#[derive(Debug)]
pub struct ObservedPopulatedGroupBundle {
    group: GenesisBundle,
    pub(super) models: Vec<GenesisBundle>,
    model_envelopes: Vec<CausalEnvelope>,
    batch_id: String,
    manifest: String,
    labels: Option<labels::Snapshot>,
    label_snapshot: Option<String>,
    project: Option<project::Snapshot>,
    project_snapshot: Option<String>,
    verified_origin: Option<SyncIdentity>,
    identity_link: Option<transport::IdentityLink>,
}
impl ObservedPopulatedGroupBundle {
    pub(super) fn identity_group(&self) -> Result<&GenesisBundle, DbError> {
        // Matching the group does not authorize merging project or label identities.
        if self.labels.is_some() || self.project.is_some() || self.identity_link.is_some() {
            return Err(invalid());
        }

        Ok(&self.group)
    }

    pub(super) async fn novel_models(
        &self,
        db: &mut SqliteConnection,
        user: &User,
    ) -> Result<(), DbError> {
        for model in &self.models {
            super::checks::empty_target(db, user, model).await?;
            // Even unresolved references are retained identity evidence. A group
            // choice cannot certify an existing child or decode old batch history.
            // Legacy group membership records name their child by a scalar id.
            let retained: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM assignment_batch_journal WHERE user_id=?1) OR EXISTS(SELECT 1 FROM causal_events WHERE user_id=?1 AND encoding='local_history_v1' AND json_extract(envelope,'$.action.model_global_id')=?2) OR EXISTS(SELECT 1 FROM causal_events e,json_tree(e.envelope) reference WHERE e.user_id=?1 AND json_extract(CASE WHEN reference.type='object' THEN reference.value ELSE '{}' END,'$.kind')='model' AND (json_extract(CASE WHEN reference.type='object' THEN reference.value ELSE '{}' END,'$.global_id')=?2 OR json_extract(CASE WHEN reference.type='object' THEN reference.value ELSE '{}' END,'$.birth_id')=?3))")
                .bind(user.id).bind(&model.entity().global_id).bind(&model.entity().birth_id).fetch_one(&mut *db).await?;
            if retained {
                return Err(invalid());
            }
        }

        Ok(())
    }

    fn receipt_source(&self) -> &SyncIdentity {
        self.verified_origin
            .as_ref()
            .unwrap_or_else(|| self.source_identity())
    }
    #[must_use]
    pub const fn group_entity(&self) -> &Entity {
        self.group.entity()
    }
    #[must_use]
    pub const fn group_event(&self) -> &Dot {
        self.group.event()
    }
    #[must_use]
    pub const fn group_envelope(&self) -> &CausalEnvelope {
        self.group.envelope()
    }
    #[must_use]
    pub fn model_envelopes(&self) -> &[CausalEnvelope] {
        &self.model_envelopes
    }
    #[must_use]
    pub const fn source_identity(&self) -> &SyncIdentity {
        self.group.source_identity()
    }
}

pub(super) async fn bind_existing_identity(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    origin: SyncIdentity,
    bundle: &mut ObservedPopulatedGroupBundle,
    group_id: i64,
    blobs: &[ModelBlobBinding],
) -> Result<(), DbError> {
    bundle.identity_group()?;
    bundle.novel_models(db, user).await?;
    bundle.verified_origin = Some(origin);
    let blob_ids = persistence::validate_blobs(db, bundle, blobs).await?;
    persistence::write_batch(db, user, identity, bundle, "installed").await?;
    for member in bundle.models.iter().chain(std::iter::once(&bundle.group)) {
        crate::causal_store::store_checked_receipt(db, user.id, &member.envelope, &member.wire)
            .await?;
    }
    persistence::install_members(db, user, identity, bundle, group_id, blob_ids).await?;

    Ok(())
}
#[derive(Debug)]
pub struct ModelBlobBinding {
    pub entity: Entity,
    pub local_blob_id: i64,
}
#[derive(Debug)]
pub struct PopulatedGroupInstallation {
    pub group_id: i64,
    pub models: Vec<(Entity, i64)>,
    pub already_present: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    kind: String,
    batch_id: String,
    source_instance_id: String,
    source_account_id: String,
    original: serde_json::Value,
    initial_group: Option<Entity>,
    members: Vec<Dependency>,
    materialization_digest: String,
    history: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label_snapshot_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_snapshot_digest: Option<String>,
}

pub async fn observe(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    group_id: i64,
) -> Result<PopulatedGroupObservation, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let observation = observe_in(&mut tx, user, identity, group_id).await?;
    tx.commit().await?;

    Ok(observation)
}

pub(super) async fn observe_in(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    group_id: i64,
) -> Result<PopulatedGroupObservation, DbError> {
    validate_local(db, user, identity).await?;
    let (group, models) = capture::read(db, user, group_id).await?;
    let labels = labels::capture(db, user, group_id).await?;
    let project = project::capture(db, user, group.resource_id).await?;

    Ok(PopulatedGroupObservation {
        owner: user.id,
        identity: identity.clone(),
        group_id,
        batch_id: crate::random_hex_32(),
        group,
        models,
        labels,
        project,
    })
}

pub async fn enroll(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    capture: &PopulatedGroupObservation,
) -> Result<ObservedPopulatedGroupBundle, DbError> {
    persistence::enroll(db, user, identity, capture, None).await
}
pub async fn install(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    peer: &PeerBinding,
    bundle: &ObservedPopulatedGroupBundle,
    blobs: &[ModelBlobBinding],
) -> Result<PopulatedGroupInstallation, DbError> {
    persistence::install(db, user, identity, peer, bundle, blobs).await
}

pub fn validate(
    root: &CausalEnvelope,
    materialization: &str,
    proof: &str,
    source: &SyncIdentity,
) -> Result<(Option<String>, Option<Entity>), DbError> {
    let proof_value: Proof = serde_json::from_str(proof).map_err(|_| invalid())?;
    if !matches!(&root.action,Action::LegacyObserved{observation_digest} if *observation_digest==observation::digest(proof))
        || root.entity.birth_id.is_none()
        || !root.parents.is_empty()
        || proof_value.history != "unknown_before_observation"
        || proof_value.source_instance_id != source.instance_id
        || proof_value.source_account_id != source.account_id
        || proof_value.original["global_id"].as_str() != Some(root.entity.global_id.as_str())
        || proof_value.original["birth_id"]
            .as_str()
            .is_some_and(|birth| Some(birth) != root.entity.birth_id.as_deref())
        || proof_value.materialization_digest != observation::digest(materialization)
    {
        return Err(invalid());
    }
    let value: Materialization = serde_json::from_str(materialization).map_err(|_| invalid())?;
    match (
        &root.entity.kind,
        &value,
        proof_value.kind.as_str(),
        proof_value.initial_group,
    ) {
        (EntityKind::Group, Materialization::Group(snapshot), "populated_group", None)
            if root.dependencies == proof_value.members =>
        {
            Ok((Some(snapshot.name.clone()), None))
        }
        (EntityKind::Model, Materialization::Model(_), "populated_model", Some(group))
            if root.dependencies.is_empty()
                && proof_value.members.is_empty()
                && group.kind == EntityKind::Group
                && group.birth_id.is_some() =>
        {
            Ok((None, Some(group)))
        }
        _ => Err(invalid()),
    }
}

/// Label state is an immutable snapshot facet until label edit events exist.
/// A changed component pauses application rather than overwriting local labels.
pub async fn labels_valid(
    db: &mut SqliteConnection,
    user: &User,
    batch_id: &str,
) -> Result<bool, DbError> {
    let (snapshot,instance_id,account_id):(Option<String>,String,String)=sqlx::query_as("SELECT label_snapshot,COALESCE(verified_source_instance,source_instance_id),COALESCE(verified_source_account,source_account_id) FROM causal_observation_batches WHERE user_id=? AND batch_id=?")
        .bind(user.id).bind(batch_id).fetch_one(&mut *db).await?;
    let digest = snapshot.as_deref().map(observation::digest);
    let mismatched:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM causal_observation_batch_members m JOIN causal_genesis_bindings b USING(user_id,entity_kind,birth_id) WHERE m.user_id=? AND m.batch_id=? AND json_extract(b.observation_evidence,'$.label_snapshot_digest') IS NOT ?)")
        .bind(user.id).bind(batch_id).bind(digest).fetch_one(&mut *db).await?;
    if mismatched {
        return Err(invalid());
    }
    let Some(snapshot) = snapshot else {
        return Ok(true);
    };
    let snapshot: labels::Snapshot = serde_json::from_str(&snapshot).map_err(|_| invalid())?;
    labels::valid(
        db,
        user,
        &SyncIdentity {
            instance_id,
            account_id,
        },
        &snapshot,
    )
    .await
}

/// Project metadata changes require a later explicit checkpoint; they are not
/// inferred from a matching project global id or silently overwritten.
pub async fn project_valid(
    db: &mut SqliteConnection,
    user: &User,
    batch_id: &str,
) -> Result<bool, DbError> {
    let (snapshot,instance_id,account_id):(Option<String>,String,String)=sqlx::query_as("SELECT project_snapshot,COALESCE(verified_source_instance,source_instance_id),COALESCE(verified_source_account,source_account_id) FROM causal_observation_batches WHERE user_id=? AND batch_id=?")
        .bind(user.id).bind(batch_id).fetch_one(&mut *db).await?;
    let digest = snapshot.as_deref().map(observation::digest);
    let mismatch:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM causal_observation_batch_members m JOIN causal_genesis_bindings b USING(user_id,entity_kind,birth_id) WHERE m.user_id=? AND m.batch_id=? AND json_extract(b.observation_evidence,'$.project_snapshot_digest') IS NOT ?)")
        .bind(user.id).bind(batch_id).bind(digest).fetch_one(&mut *db).await?;
    if mismatch {
        return Err(invalid());
    }
    let Some(snapshot) = snapshot else {
        return Ok(true);
    };
    let snapshot: project::Snapshot = serde_json::from_str(&snapshot).map_err(|_| invalid())?;
    project::valid(
        db,
        user,
        &SyncIdentity {
            instance_id,
            account_id,
        },
        &snapshot,
    )
    .await
}
