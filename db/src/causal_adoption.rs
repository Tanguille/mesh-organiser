//! Guarded observations of edited local state. This is an in-process checkpoint,
//! not authenticated synchronization or identity merging between legacy peers.

use crate::{
    DbError, db_context::DbContext, model::user::User, sync_peer::PeerBinding,
    user_db::SyncIdentity,
};

pub use crate::causal_genesis::observation::GroupObservation;

#[derive(Debug)]
pub struct ObservedGroupBundle {
    inner: crate::causal_genesis::GenesisBundle,
}

impl ObservedGroupBundle {
    #[must_use]
    pub const fn entity(&self) -> &crate::causal_envelope::Entity {
        self.inner.entity()
    }

    #[must_use]
    pub const fn event(&self) -> &crate::causal_envelope::Dot {
        self.inner.event()
    }

    #[must_use]
    pub const fn envelope(&self) -> &crate::causal_envelope::CausalEnvelope {
        self.inner.envelope()
    }

    #[must_use]
    pub const fn source_identity(&self) -> &SyncIdentity {
        self.inner.source_identity()
    }
}

pub async fn observe_group(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    group_id: i64,
) -> Result<GroupObservation, DbError> {
    crate::causal_genesis::observation::observe(db, user, identity, group_id).await
}

pub async fn enroll_observed_group(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    observation: &GroupObservation,
) -> Result<ObservedGroupBundle, DbError> {
    let inner = crate::causal_genesis::observation::enroll(db, user, identity, observation).await?;
    Ok(ObservedGroupBundle { inner })
}

pub async fn install_observed_group(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    peer: &PeerBinding,
    bundle: &ObservedGroupBundle,
) -> Result<crate::causal_genesis::GenesisInstallation, DbError> {
    crate::causal_genesis::install_pristine_genesis(db, user, identity, peer, &bundle.inner, None)
        .await
}

pub use crate::causal_genesis::populated::{
    ModelBlobBinding, ObservedPopulatedGroupBundle, PopulatedGroupInstallation,
    PopulatedGroupObservation,
};

pub async fn observe_populated_group(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    group_id: i64,
) -> Result<PopulatedGroupObservation, DbError> {
    crate::causal_genesis::populated::observe(db, user, identity, group_id).await
}
pub async fn enroll_observed_populated_group(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    capture: &PopulatedGroupObservation,
) -> Result<ObservedPopulatedGroupBundle, DbError> {
    crate::causal_genesis::populated::enroll(db, user, identity, capture).await
}
pub async fn install_observed_populated_group(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    peer: &PeerBinding,
    bundle: &ObservedPopulatedGroupBundle,
    blobs: &[ModelBlobBinding],
) -> Result<PopulatedGroupInstallation, DbError> {
    crate::causal_genesis::populated::install(db, user, identity, peer, bundle, blobs).await
}
