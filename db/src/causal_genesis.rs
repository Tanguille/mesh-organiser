//! Internal checked copies of pristine genesis, not authenticated network sync.
//! Edited/legacy libraries require a separate observation/adoption protocol.

use crate::{
    DbError,
    causal_envelope::{CausalEnvelope, Dot, Entity, EntityKind},
    db_context::DbContext,
    model::user::User,
    sync_peer::{validate_identity, validate_local},
    user_db::SyncIdentity,
};

mod checks;
pub mod identity;
mod materialization;
pub(crate) mod observation;
pub(crate) mod populated;
mod storage;
pub(crate) mod transport;

const MAX_MATERIALIZATION_BYTES: i64 = 2 * 1024 * 1024;

/// Opaque evidence checked against a real source database. There is deliberately no
/// wire deserializer: peer pins alone are not authentication or blob verification.
#[derive(Debug)]
pub struct GenesisBundle {
    envelope: CausalEnvelope,
    wire: String,
    source_identity: SyncIdentity,
    materialization: materialization::Materialization,
    materialization_json: String,
    capability: String,
    observation_evidence: Option<String>,
}

impl GenesisBundle {
    #[must_use]
    pub const fn envelope(&self) -> &CausalEnvelope {
        &self.envelope
    }

    #[must_use]
    pub const fn entity(&self) -> &Entity {
        &self.envelope.entity
    }

    #[must_use]
    pub const fn event(&self) -> &Dot {
        &self.envelope.event
    }

    #[must_use]
    pub const fn source_identity(&self) -> &SyncIdentity {
        &self.source_identity
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct GenesisInstallation {
    pub local_id: i64,
    /// A receipt acknowledgement, not proof that the entity is still pristine.
    pub already_present: bool,
}

fn invalid() -> DbError {
    DbError::Protocol("Invalid or unsupported pristine genesis".into())
}

const fn kind(entity: &Entity) -> &'static str {
    match entity.kind {
        EntityKind::Group => "group",
        EntityKind::Model => "model",
    }
}

fn counter(event: &Dot) -> Result<i64, DbError> {
    event.counter.parse().map_err(|_| invalid())
}

fn encode(value: &impl serde::Serialize) -> Result<String, DbError> {
    let value = serde_json::to_string(value).map_err(|_| invalid())?;
    if value.len() > usize::try_from(MAX_MATERIALIZATION_BYTES).map_err(|_| invalid())? {
        return Err(invalid());
    }

    Ok(value)
}

pub async fn enroll_pristine_group(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    group_id: i64,
) -> Result<GenesisBundle, DbError> {
    enroll(
        db,
        user,
        identity,
        EntityKind::Group,
        group_id,
        "names_moves_v1",
    )
    .await
}

pub async fn enroll_pristine_model(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    model_id: i64,
) -> Result<GenesisBundle, DbError> {
    enroll(
        db,
        user,
        identity,
        EntityKind::Model,
        model_id,
        "names_moves_v1",
    )
    .await
}

pub(crate) async fn enroll(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    entity_kind: EntityKind,
    id: i64,
    capability: &str,
) -> Result<GenesisBundle, DbError> {
    validate_identity(identity)?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut tx, user, identity).await?;
    let (entity, materialization, guard) = match entity_kind {
        EntityKind::Group => materialization::read_group(&mut tx, user, id).await?,
        EntityKind::Model => materialization::read_model(&mut tx, user, id).await?,
    };
    let (envelope, wire) = checks::genuine_root(&mut tx, user, &entity, &materialization).await?;
    checks::no_other_history(&mut tx, user, &entity, &envelope.event).await?;
    checks::uncontested(&mut tx, user, &entity).await?;
    let materialization_json = encode(&materialization)?;
    let bundle = GenesisBundle {
        envelope,
        wire,
        source_identity: identity.clone(),
        materialization,
        materialization_json,
        capability: capability.into(),
        observation_evidence: None,
    };
    if let Some(prior) = storage::binding(&mut tx, user, bundle.entity()).await? {
        prior.compare(identity, &bundle, "local")?;
        if prior.local_id != id || prior.local_guard != encode(&guard)? {
            return Err(invalid());
        }
    } else {
        storage::write_binding(&mut tx, user, identity, &bundle, id, &guard, "local").await?;
    }
    tx.commit().await?;

    Ok(bundle)
}

/// Installs an opaque in-process bundle only. Do not wrap this in a wire API without
/// authenticated source/account and verified file-transfer protocols.
pub async fn install_pristine_genesis(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    peer: &crate::sync_peer::PeerBinding,
    bundle: &GenesisBundle,
    local_blob_id: Option<i64>,
) -> Result<GenesisInstallation, DbError> {
    validate_identity(identity)?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut tx, user, identity).await?;
    checks::peer(&mut tx, user, identity, peer, bundle).await?;
    materialization::validate_blob(&mut tx, &bundle.materialization, local_blob_id).await?;
    if let Some(prior) = storage::binding(&mut tx, user, bundle.entity()).await? {
        prior.compare(identity, bundle, "installed")?;
        let local_id =
            storage::validate_retry(&mut tx, user, bundle, &prior, local_blob_id).await?;
        tx.commit().await?;

        return Ok(GenesisInstallation {
            local_id,
            already_present: true,
        });
    }
    checks::empty_target(&mut tx, user, bundle).await?;
    storage::store_received_root(&mut tx, user, bundle).await?;
    let (local_id, guard) = materialization::install(
        &mut tx,
        user,
        bundle.entity(),
        &bundle.materialization,
        local_blob_id,
    )
    .await?;
    storage::write_binding(
        &mut tx,
        user,
        identity,
        bundle,
        local_id,
        &guard,
        "installed",
    )
    .await?;
    // Received genesis is a scoped observation, never a forged local creation record.
    // Its entity_births initialization status intentionally remains unknown.
    tx.commit().await?;

    Ok(GenesisInstallation {
        local_id,
        already_present: false,
    })
}
