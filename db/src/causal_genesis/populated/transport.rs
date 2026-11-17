//! Strict wire decoding does not authenticate its claimed source identity.
use super::{
    GenesisBundle, ObservedPopulatedGroupBundle, Proof, encode, invalid, labels,
    materialization::Materialization, observation, persistence, project,
};
use crate::{
    DbError,
    causal_envelope::{self, Dependency, EntityKind},
    model::user::User,
    user_db::SyncIdentity,
};
use sqlx::SqliteConnection;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobContent {
    pub hash: String,
    pub filetype: String,
    pub size: i64,
    pub sha256: String,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityLink {
    pub observation_id: String,
    pub baseline: causal_envelope::Dot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Dependency>,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    envelope: String,
    materialization: String,
    proof: String,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    kind: String,
    batch_id: String,
    source_identity: SyncIdentity,
    group: Member,
    models: Vec<Member>,
    manifest: String,
    label_snapshot: Option<String>,
    project_snapshot: Option<String>,
    contents: Vec<BlobContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity_link: Option<IdentityLink>,
}
fn member(bundle: &GenesisBundle) -> Result<Member, DbError> {
    Ok(Member {
        envelope: bundle.wire.clone(),
        materialization: bundle.materialization_json.clone(),
        proof: bundle.observation_evidence.clone().ok_or_else(invalid)?,
    })
}
pub fn encode_bundle(
    bundle: &ObservedPopulatedGroupBundle,
    contents: Vec<BlobContent>,
) -> Result<serde_json::Value, DbError> {
    serde_json::to_value(Wire {
        kind: "observed_group".into(),
        batch_id: bundle.batch_id.clone(),
        source_identity: bundle.source_identity().clone(),
        group: member(&bundle.group)?,
        models: bundle.models.iter().map(member).collect::<Result<_, _>>()?,
        manifest: bundle.manifest.clone(),
        label_snapshot: bundle.label_snapshot.clone(),
        project_snapshot: bundle.project_snapshot.clone(),
        contents,
        identity_link: bundle.identity_link.clone(),
    })
    .map_err(|_| invalid())
}
fn decode_member(
    value: Member,
    source: &SyncIdentity,
    batch: &str,
) -> Result<GenesisBundle, DbError> {
    let envelope =
        causal_envelope::parse_causal_envelope(&value.envelope).map_err(|_| invalid())?;
    let materialization: Materialization =
        serde_json::from_str(&value.materialization).map_err(|_| invalid())?;
    let proof: Proof = serde_json::from_str(&value.proof).map_err(|_| invalid())?;
    if causal_envelope::canonical_causal_envelope(&envelope).map_err(|_| invalid())?
        != value.envelope
        || encode(&materialization)? != value.materialization
        || proof.batch_id != batch
    {
        return Err(invalid());
    }
    super::validate(&envelope, &value.materialization, &value.proof, source)?;

    Ok(GenesisBundle {
        envelope,
        wire: value.envelope,
        source_identity: source.clone(),
        materialization,
        materialization_json: value.materialization,
        capability: "group_lifecycle_v1".into(),
        observation_evidence: Some(value.proof),
    })
}
pub fn decode(
    value: serde_json::Value,
) -> Result<(ObservedPopulatedGroupBundle, Vec<BlobContent>), DbError> {
    let wire: Wire = serde_json::from_value(value).map_err(|_| invalid())?;
    crate::sync_peer::validate_identity(&wire.source_identity)?;
    if wire.kind != "observed_group"
        || wire.batch_id.len() != 32
        || wire.models.len() > causal_envelope::MAX_CONTEXT_ITEMS
    {
        return Err(invalid());
    }
    let group = decode_member(wire.group, &wire.source_identity, &wire.batch_id)?;
    let models = wire
        .models
        .into_iter()
        .map(|member| decode_member(member, &wire.source_identity, &wire.batch_id))
        .collect::<Result<Vec<_>, _>>()?;
    if group.entity().kind != EntityKind::Group {
        return Err(invalid());
    }
    let mut globals = BTreeSet::new();
    for model in &models {
        let (_, initial) = super::validate(
            &model.envelope,
            &model.materialization_json,
            model.observation_evidence.as_deref().ok_or_else(invalid)?,
            &wire.source_identity,
        )?;
        if model.entity().kind != EntityKind::Model
            || initial.as_ref() != Some(group.entity())
            || !globals.insert(model.entity().global_id.clone())
        {
            return Err(invalid());
        }
    }
    let references: Vec<_> = models
        .iter()
        .chain(std::iter::once(&group))
        .map(|member| Dependency {
            entity: member.entity().clone(),
            event: member.event().clone(),
        })
        .collect();
    if encode(&references)? != wire.manifest
        || group.envelope.dependencies != references[..models.len()]
    {
        return Err(invalid());
    }
    let labels: Option<labels::Snapshot> = wire
        .label_snapshot
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| invalid())?;
    let project: Option<project::Snapshot> = wire
        .project_snapshot
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| invalid())?;
    for member in models.iter().chain(std::iter::once(&group)) {
        let proof: Proof =
            serde_json::from_str(member.observation_evidence.as_deref().ok_or_else(invalid)?)
                .map_err(|_| invalid())?;
        if proof.label_snapshot_digest != wire.label_snapshot.as_deref().map(observation::digest)
            || proof.project_snapshot_digest
                != wire.project_snapshot.as_deref().map(observation::digest)
        {
            return Err(invalid());
        }
    }
    if let Some(labels) = &labels {
        validate_labels(labels, &models)?;
        if Some(encode(labels)?) != wire.label_snapshot {
            return Err(invalid());
        }
    }
    if let Some(project) = &project
        && (project.incarnation.is_empty()
            || project.global_id.is_empty()
            || Some(encode(project)?) != wire.project_snapshot)
    {
        return Err(invalid());
    }
    let model_envelopes = models.iter().map(|model| model.envelope.clone()).collect();
    let bundle = ObservedPopulatedGroupBundle {
        group,
        models,
        model_envelopes,
        batch_id: wire.batch_id,
        manifest: wire.manifest,
        labels,
        label_snapshot: wire.label_snapshot,
        project,
        project_snapshot: wire.project_snapshot,
        verified_origin: None,
        identity_link: wire.identity_link,
    };

    if bundle.identity_link.is_some() {
        super::identity::observation(&bundle)?.validate()?;
        super::model_identity::validate_link(&bundle)?;
    }
    Ok((bundle, wire.contents))
}
fn validate_labels(snapshot: &labels::Snapshot, models: &[GenesisBundle]) -> Result<(), DbError> {
    let mut incarnations = BTreeSet::new();
    let mut globals = BTreeSet::new();
    if snapshot.nodes.len() > 4096
        || snapshot.edges.len() > 32768
        || snapshot.assignments.len() > 32768
    {
        return Err(invalid());
    }
    for node in &snapshot.nodes {
        if node.incarnation.is_empty()
            || node.global_id.is_empty()
            || !incarnations.insert(&node.incarnation)
            || !globals.insert(&node.global_id)
        {
            return Err(invalid());
        }
    }
    for edge in &snapshot.edges {
        if !incarnations.contains(&edge.parent) || !incarnations.contains(&edge.child) {
            return Err(invalid());
        }
    }
    for assignment in &snapshot.assignments {
        if !incarnations.contains(&assignment.label)
            || !models
                .iter()
                .any(|model| model.entity() == &assignment.model)
        {
            return Err(invalid());
        }
    }
    Ok(())
}
pub fn descriptors(
    bundle: &ObservedPopulatedGroupBundle,
) -> Result<Vec<(causal_envelope::Entity, BlobContent)>, DbError> {
    bundle
        .models
        .iter()
        .map(|model| match &model.materialization {
            Materialization::Model(snapshot) => Ok((
                model.entity().clone(),
                BlobContent {
                    hash: snapshot.blob.hash.clone(),
                    filetype: snapshot.blob.filetype.clone(),
                    size: snapshot.blob.size,
                    sha256: String::new(),
                },
            )),
            Materialization::Group(_) => Err(invalid()),
        })
        .collect()
}
pub fn roots(bundle: &ObservedPopulatedGroupBundle) -> Vec<&causal_envelope::CausalEnvelope> {
    bundle
        .models
        .iter()
        .chain(std::iter::once(&bundle.group))
        .map(GenesisBundle::envelope)
        .collect()
}
pub fn batch_id(bundle: &ObservedPopulatedGroupBundle) -> &str {
    &bundle.batch_id
}
pub async fn install(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    origin: &SyncIdentity,
    bundle: &mut ObservedPopulatedGroupBundle,
    blobs: &[super::ModelBlobBinding],
) -> Result<super::PopulatedGroupInstallation, DbError> {
    bundle.verified_origin = Some(origin.clone());
    if bundle.identity_link.is_some() {
        return super::identity::install(db, user, identity, bundle, blobs).await;
    }
    persistence::install_transaction(db, user, identity, bundle, blobs).await
}

/// Verify an installed baseline against its newly authenticated original.
///
/// The service has verified both incoming and existing logical bytes;
/// this function preserves the current projection and its portable branches.
pub async fn retain_installed_baseline(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    bundle: &ObservedPopulatedGroupBundle,
    contents: &[BlobContent],
    blobs: &[super::ModelBlobBinding],
) -> Result<bool, DbError> {
    let stored:Option<(String,String,String)>=sqlx::query_as("SELECT local_instance_id,local_account_id,origin FROM causal_observation_batches WHERE user_id=? AND batch_id=?")
        .bind(user.id).bind(&bundle.batch_id).fetch_optional(&mut *db).await?;
    let Some((instance, account, origin)) = stored else {
        return Ok(false);
    };
    if instance != identity.instance_id || account != identity.account_id || origin != "installed" {
        return Err(invalid());
    }
    let mut archived = read(db, user, &bundle.batch_id).await?;
    let incoming = encode_bundle(bundle, contents.to_vec())?;
    if archived["contents"] == serde_json::json!([]) {
        // In-process checked installations never claimed a verified full digest.
        // Only the service's real file checks allow first-time content attestation.
        archived["contents"] = incoming["contents"].clone();
    }
    if archived != incoming || blobs.len() != bundle.models.len() {
        return Err(invalid());
    }
    crate::causal_group_lifecycle::validate_installed_baseline(db, user, &roots(bundle)).await?;
    for model in &bundle.models {
        let matches = blobs
            .iter()
            .filter(|blob| blob.entity == *model.entity())
            .collect::<Vec<_>>();
        let [mapping] = matches.as_slice() else {
            return Err(invalid());
        };
        let bound:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM causal_genesis_bindings b JOIN models m ON m.model_id=b.local_id AND m.model_user_id=b.user_id WHERE b.user_id=? AND b.entity_kind='model' AND b.birth_id=? AND b.global_id=? AND m.model_birth_id=b.birth_id AND m.model_unique_global_id=b.global_id AND m.model_blob_id=?)")
            .bind(user.id).bind(&model.entity().birth_id).bind(&model.entity().global_id).bind(mapping.local_blob_id).fetch_one(&mut *db).await?;
        if !bound {
            return Err(invalid());
        }
    }
    Ok(true)
}

pub async fn read(
    db: &mut SqliteConnection,
    user: &User,
    batch: &str,
) -> Result<serde_json::Value, DbError> {
    let (instance,account,manifest,labels,project,identity_link):(String,String,String,Option<String>,Option<String>,Option<String>)=sqlx::query_as("SELECT source_instance_id,source_account_id,manifest,label_snapshot,project_snapshot,identity_link FROM causal_observation_batches WHERE user_id=? AND batch_id=?").bind(user.id).bind(batch).fetch_one(&mut *db).await?;
    let references: Vec<Dependency> = serde_json::from_str(&manifest).map_err(|_| invalid())?;
    let mut members = Vec::new();
    for reference in &references {
        let primary:Option<(String,String,String)>=sqlx::query_as("SELECT e.envelope,b.materialization,b.observation_evidence FROM causal_genesis_bindings b JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.writer_id=? AND b.counter=?").bind(user.id).bind(&reference.event.writer_id).bind(reference.event.counter.parse::<i64>().map_err(|_| invalid())?).fetch_optional(&mut *db).await?;
        let (envelope, materialization, proof) = if let Some(primary) = primary {
            primary
        } else {
            sqlx::query_as("SELECT json_extract(item,'$.observation.root'),json_extract(item,'$.observation.materialization'),json_extract(item,'$.observation.proof') FROM group_identity_observations WHERE user_id=?1 AND batch_id=?2 AND writer_id=?3 AND counter=?4 UNION ALL SELECT root,materialization,proof FROM model_identity_observations WHERE user_id=?1 AND batch_id=?2 AND writer_id=?3 AND counter=?4")
                .bind(user.id).bind(batch).bind(&reference.event.writer_id).bind(reference.event.counter.parse::<i64>().map_err(|_| invalid())?).fetch_one(&mut *db).await?
        };
        members.push(Member {
            envelope,
            materialization,
            proof,
        });
    }
    let group = members.pop().ok_or_else(invalid)?;
    let contents: Vec<BlobContent> = sqlx::query_as::<_,(String,String,i64,String)>("SELECT hash,filetype,size,sha256 FROM sync_observation_contents WHERE user_id=? AND batch_id=? ORDER BY hash").bind(user.id).bind(batch).fetch_all(db).await?.into_iter().map(|(hash,filetype,size,sha256)|BlobContent{hash,filetype,size,sha256}).collect();

    serde_json::to_value(Wire {
        kind: "observed_group".into(),
        batch_id: batch.into(),
        source_identity: SyncIdentity {
            instance_id: instance,
            account_id: account,
        },
        group,
        models: members,
        manifest,
        label_snapshot: labels,
        project_snapshot: project,
        identity_link: identity_link
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| invalid())?,
        contents,
    })
    .map_err(|_| invalid())
}

/// Re-export only an unchanged original local observation, never a new baseline.
pub async fn reload_source(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    group_id: i64,
) -> Result<Option<ObservedPopulatedGroupBundle>, DbError> {
    let batch:Option<String>=sqlx::query_scalar("SELECT m.batch_id FROM causal_observation_batch_members m JOIN causal_genesis_bindings b USING(user_id,entity_kind,birth_id) WHERE b.user_id=? AND b.entity_kind='group' AND b.local_id=? AND b.origin='local'")
        .bind(user.id).bind(group_id).fetch_optional(&mut *db).await?;
    let Some(batch) = batch else {
        return Ok(None);
    };
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM causal_observation_batch_members WHERE user_id=? AND batch_id=? AND entity_kind='model'").bind(user.id).bind(&batch).fetch_one(&mut *db).await?;
    let count = usize::try_from(count).map_err(|_| invalid())?;
    if count > causal_envelope::MAX_CONTEXT_ITEMS {
        return Err(invalid());
    }

    persistence::load_retry(db, user, identity, &batch, count).await
}

pub async fn enroll_source(
    db: &crate::db_context::DbContext,
    user: &User,
    identity: &SyncIdentity,
    capture: &super::PopulatedGroupObservation,
    contents: &[BlobContent],
) -> Result<ObservedPopulatedGroupBundle, DbError> {
    persistence::enroll(db, user, identity, capture, Some(contents)).await
}

pub async fn enroll_source_in(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    capture: &super::PopulatedGroupObservation,
    contents: &[BlobContent],
) -> Result<ObservedPopulatedGroupBundle, DbError> {
    persistence::enroll_in(db, user, identity, capture, Some(contents)).await
}
