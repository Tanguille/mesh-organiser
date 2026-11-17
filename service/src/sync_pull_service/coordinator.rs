//! One captured feed item and explicitly selected publications; no automatic account scheduler.
pub use db::sync_transport::operation_selection::OperationSelection;
use db::{
    causal_genesis::identity, model::user::User, sync_transport::outbox, user_db::SyncIdentity,
};

use crate::{AppState, ServiceError};

use super::{NextDelivery, PendingPublications, PullOutcome, invalid};

/// A feed boundary is independent from pending publications and unresolved conflicts.
#[derive(Debug)]
pub enum AdvanceOutcome {
    IdentityRequired(super::GroupIdentityChoice),
    Applied { sequence: String },
    AlreadyApplied { sequence: String },
    FeedBoundary { through: String },
}

impl super::AuthenticatedHub {
    pub async fn select_next_operation(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
    ) -> Result<OperationSelection, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }
        let db = state.db.clone();
        let authorization = self.authorization.clone();
        // A lost caller cannot separate the selected original from its durable disposition.
        Ok(tokio::spawn(async move {
            db::sync_transport::operation_selection::select_next(&db, &authorization).await
        })
        .await
        .map_err(|_| invalid())??)
    }

    pub async fn advance_one(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
    ) -> Result<AdvanceOutcome, ServiceError> {
        tokio::time::timeout(
            std::time::Duration::from_secs(120),
            self.advance(state, user, identity),
        )
        .await
        .map_err(|_| invalid())?
    }

    async fn advance(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
    ) -> Result<AdvanceOutcome, ServiceError> {
        let (delivery, item) = match self.next_delivery(state, user, identity).await? {
            NextDelivery::Idle(through) => return Ok(AdvanceOutcome::FeedBoundary { through }),
            NextDelivery::Item(delivery, item) => (*delivery, item),
        };
        // An exact certified echo describes history, not today's candidate row.
        // Retain the puller's proof checks before considering any identity question.
        if let Some(outcome) = self.acknowledge_captured(state, &delivery, &item).await? {
            return advance_outcome(outcome);
        }
        if let Some(choice) =
            identity::model_pair::prepare_next(&state.db, &delivery, &item).await?
        {
            return Ok(AdvanceOutcome::IdentityRequired(choice));
        }
        if let Some(choice) = identity::prepare_next(&state.db, &delivery, &item).await? {
            return Ok(AdvanceOutcome::IdentityRequired(choice));
        }
        // The fetched item is the unit of work. Fetching again after inspection
        // could skip or substitute a delivery when another caller advances it.
        advance_outcome(self.apply_captured(state, user, delivery, item).await?)
    }

    pub async fn pending_publications(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
    ) -> Result<PendingPublications, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }

        Ok(outbox::pending(&state.db, &self.authorization).await?)
    }

    pub async fn pending_publications_page(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        cursor: Option<&str>,
    ) -> Result<PendingPublications, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }

        Ok(outbox::pending_page(&state.db, &self.authorization, cursor).await?)
    }
}

fn advance_outcome(outcome: PullOutcome) -> Result<AdvanceOutcome, ServiceError> {
    match outcome {
        PullOutcome::Applied { sequence } => Ok(AdvanceOutcome::Applied { sequence }),
        PullOutcome::AlreadyApplied { sequence } => Ok(AdvanceOutcome::AlreadyApplied { sequence }),
        PullOutcome::Idle { .. } => Err(invalid()),
    }
}
