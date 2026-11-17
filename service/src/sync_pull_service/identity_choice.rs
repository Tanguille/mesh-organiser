//! Explicit legacy identity confirmation; production synchronization remains gated.
use super::{AuthenticatedHub, NextDelivery, PublicationOutcome, invalid};
use crate::{
    AppState, ServiceError,
    sync_observation_service::{self, IncomingObservation},
};
use db::causal_genesis::identity::{self, Decision, DecisionOutcome, IdentityTransfer};
pub use db::causal_genesis::identity::{
    GroupIdentityChoice, GroupIdentityOutcome, GroupSeparationOutcome, NamePublication,
    ReceivedNameChoice,
};
use db::sync_transport::{self, pull::Delivery};
use db::{causal_envelope::Dot, model::user::User, user_db::SyncIdentity};

impl AuthenticatedHub {
    pub async fn prepare_received_group_name(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        group_id: i64,
    ) -> Result<ReceivedNameChoice, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }

        Ok(identity::prepare_received_name(&state.db, &self.authorization, group_id).await?)
    }

    pub async fn resolve_received_group_name(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        choice_id: &str,
        name: &str,
    ) -> Result<NamePublication, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }
        let db = state.db.clone();
        let auth = self.authorization.clone();
        let choice_id = choice_id.to_owned();
        let name = name.to_owned();
        // Cancellation cannot strand a committed resolution without its outbox.
        Ok(tokio::spawn(async move {
            identity::resolve_received_name(&db, &auth, &choice_id, &name).await
        })
        .await
        .map_err(|_| invalid())??)
    }

    pub async fn resolve_group_identity_name(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        decision_id: &str,
        token: &db::causal_group_rename::NameToken,
        name: &str,
    ) -> Result<NamePublication, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }
        let db = state.db.clone();
        let auth = self.authorization.clone();
        let decision_id = decision_id.to_owned();
        let token = token.clone();
        let name = name.to_owned();
        // A lost caller cannot interrupt resolution between its event and outbox.
        Ok(tokio::spawn(async move {
            identity::resolve_name_publication(&db, &auth, &decision_id, &token, &name).await
        })
        .await
        .map_err(|_| invalid())??)
    }

    pub async fn separate_group_identity(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        decision_id: &str,
    ) -> Result<GroupSeparationOutcome, ServiceError> {
        match self
            .decide_group_identity(state, user, identity, decision_id, Decision::Separate)
            .await?
        {
            DecisionOutcome::Separate(result) => Ok(result),
            _ => Err(invalid()),
        }
    }
    pub async fn prepare_group_identity(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        local_group_id: i64,
    ) -> Result<GroupIdentityChoice, ServiceError> {
        let NextDelivery::Item(delivery, item) = self.next_delivery(state, user, identity).await?
        else {
            return Err(invalid());
        };
        Ok(identity::prepare(&state.db, *delivery, item, local_group_id).await?)
    }

    pub async fn confirm_group_identity(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        decision_id: &str,
    ) -> Result<GroupIdentityOutcome, ServiceError> {
        match self
            .decide_group_identity(state, user, identity, decision_id, Decision::Same)
            .await?
        {
            DecisionOutcome::Same(result) => Ok(result),
            _ => Err(invalid()),
        }
    }

    pub(super) async fn decide_group_identity(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        decision_id: &str,
        decision: Decision,
    ) -> Result<DecisionOutcome, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }
        let (delivery, item, local_group_id) = match Box::pin(identity::prepare_transfer(
            &state.db,
            &self.authorization,
            decision_id,
            decision,
        ))
        .await?
        {
            IdentityTransfer::Complete(result) => return Ok(result),
            IdentityTransfer::Pending {
                delivery,
                item,
                local_group_id,
            } => (*delivery, item, local_group_id),
        };
        let plan = sync_transport::observations::parse(&item)?;
        if plan.contents().is_empty() && local_group_id.is_none() {
            let state = state.clone();
            let authorization = self.authorization.clone();
            let decision_id = decision_id.to_owned();
            return Ok(tokio::spawn(async move {
                match decision {
                    Decision::ModelPair(_) => unreachable!("model pair has a local model stage"),
                    Decision::Same => identity::confirm(&state.db, &authorization, &decision_id)
                        .await
                        .map(DecisionOutcome::Same),
                    Decision::Separate => {
                        identity::separate(&state.db, &authorization, &decision_id)
                            .await
                            .map(DecisionOutcome::Separate)
                    }
                }
            })
            .await
            .map_err(|_| invalid())??);
        }
        let local_source = if let Some(group_id) = local_group_id {
            Some(
                sync_observation_service::prepare_identity_source(state, user, identity, group_id)
                    .await?,
            )
        } else {
            None
        };
        let incoming = self.download_observation(state, &delivery, &item).await?;
        let user = user.clone();
        let state = state.clone();
        let decision_id = decision_id.to_owned();
        // Retain the lease through the whole blob/identity/receipt commit even
        // if the waiting caller disconnects after confirming their decision.
        tokio::spawn(async move {
            sync_observation_service::install_identity_delivery(
                state,
                user,
                delivery,
                decision_id,
                decision,
                item,
                incoming,
                local_source,
            )
            .await
        })
        .await
        .map_err(|_| invalid())?
    }

    pub(super) async fn download_observation(
        &self,
        state: &AppState,
        delivery: &Delivery,
        item: &serde_json::Value,
    ) -> Result<IncomingObservation, ServiceError> {
        let sequence = delivery.sequence;
        let plan = sync_transport::observations::parse(item)?;
        let mut incoming = IncomingObservation::new(state, &plan).await?;
        for content in plan.contents() {
            let mut response = self
                .client
                .get(format!(
                    "{}/api/v2/sync/observations/{sequence}/blobs/{}",
                    self.authorization.origin, content.sha256
                ))
                .header("x-sync-writer-secret", self.authorization.secret())
                .query(&[
                    ("protocol", "1"),
                    ("connection_id", self.authorization.connection.as_str()),
                ])
                .send()
                .await?;
            if response.status() != reqwest::StatusCode::OK {
                return Err(invalid());
            }
            let mut blob = incoming.start_blob(&content.sha256).await?;
            loop {
                let chunk = response.chunk().await?;
                let Some(chunk) = chunk else { break };
                blob.write(&chunk).await?;
            }
            incoming.finish_blob(blob).await?;
        }
        Ok(incoming)
    }

    pub async fn publish_group_identity(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        event: &Dot,
    ) -> Result<PublicationOutcome, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }
        let item = identity::export(&state.db, &self.authorization, event).await?;
        self.publish_item(state, user, identity, event, &item).await
    }
}
