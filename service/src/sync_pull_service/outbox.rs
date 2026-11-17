use db::{
    model::user::User,
    sync_transport::{self, outbox},
    user_db::SyncIdentity,
};

use crate::{AppState, ServiceError, sync_observation_service};

use super::{PublicationOutcome, invalid};

impl super::AuthenticatedHub {
    pub async fn queue_observation(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        group_id: i64,
    ) -> Result<String, ServiceError> {
        sync_observation_service::queue_observed_group(
            state,
            user,
            identity,
            &self.authorization,
            group_id,
        )
        .await
    }
    pub async fn queue_operation(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        event: &db::causal_envelope::Dot,
    ) -> Result<String, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }
        let bundle =
            db::causal_group_lifecycle::export_operation(&state.db, user, identity, event).await?;
        let item = sync_transport::encode_operation(&bundle)?;
        Ok(outbox::queue(&state.db, &self.authorization, event, &item).await?)
    }
    pub async fn queue_genesis(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        bundle: &db::causal_genesis::GenesisBundle,
    ) -> Result<String, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }
        let item = sync_transport::encode_genesis(bundle)?;
        Ok(outbox::queue(&state.db, &self.authorization, bundle.event(), &item).await?)
    }
    #[allow(clippy::significant_drop_tightening)] // The lease moves into the owned ACK/cleanup finalizer.
    pub async fn publish_queued(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        outbox_id: &str,
    ) -> Result<PublicationOutcome, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }
        let queued = outbox::load(&state.db, &self.authorization, outbox_id).await?;
        if let Some(sequence) = queued.ack_sequence {
            // A previous commit can have outlived its HTTP caller. Cleanup is
            // recoverable, and does not require republishing acknowledged data.
            sync_observation_service::recover_transfers(state).await?;

            return Ok(PublicationOutcome::AlreadyPresent { sequence });
        }
        let observed =
            queued.item.get("kind").and_then(serde_json::Value::as_str) == Some("observed_group");
        let mut prepared = if observed && queued.stage.is_some() {
            Some(sync_observation_service::resume_queued(state, &queued).await?)
        } else {
            None
        };
        let result = if let Some(prepared) = &prepared {
            self.publish_observation(state, user, identity, prepared)
                .await?
        } else if observed {
            self.publish_empty_observation(state, user, identity, &queued.event, &queued.item)
                .await?
        } else {
            self.publish_item(state, user, identity, &queued.event, &queued.item)
                .await?
        };
        let sequence = match &result {
            PublicationOutcome::Published { sequence }
            | PublicationOutcome::AlreadyPresent { sequence } => sequence.clone(),
            PublicationOutcome::NeedsDependencies { .. } => return Ok(result),
        };
        let state = state.clone();
        let authorization = self.authorization.clone();
        // Keep the lease in an owned finalizer across the database ACK and stage
        // release. A disconnected caller retries the same immutable outbox id.
        tokio::spawn(async move {
            outbox::acknowledge(&state.db, &authorization, &queued.id, &sequence).await?;
            #[cfg(all(test, unix))]
            sync_observation_service::outbox_acknowledged()?;
            if let Some(prepared) = &mut prepared {
                prepared.finish_queued(&state).await?;
            }
            Ok::<_, ServiceError>(())
        })
        .await
        .map_err(|_| invalid())??;

        Ok(result)
    }
}
