//! Explicit model identity answers after a token-bound group choice.
use super::{AuthenticatedHub, invalid};
use crate::{AppState, ServiceError};
use db::causal_genesis::identity::{self, Decision, DecisionOutcome};
use db::{model::user::User, user_db::SyncIdentity};
pub use identity::model_pair::{
    GroupIdentityDecision, ModelIdentityChoice, ModelIdentityDecision, ModelIdentityOutcome,
};
impl AuthenticatedHub {
    pub async fn cancel_group_identity_choice(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        decision_id: &str,
    ) -> Result<(), ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }
        let db = state.db.clone();
        let auth = self.authorization.clone();
        let decision_id = decision_id.to_owned();
        Ok(
            tokio::spawn(async move { identity::cancel(&db, &auth, &decision_id).await })
                .await
                .map_err(|_| invalid())??,
        )
    }

    pub async fn prepare_group_model_identity(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        group_decision_id: &str,
        local_model_id: i64,
        group_decision: GroupIdentityDecision,
    ) -> Result<ModelIdentityChoice, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }
        Ok(identity::model_pair::prepare(
            &state.db,
            &self.authorization,
            group_decision_id,
            local_model_id,
            group_decision,
        )
        .await?)
    }
    pub async fn confirm_group_model_identity(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        decision_id: &str,
        decision: ModelIdentityDecision,
    ) -> Result<ModelIdentityOutcome, ServiceError> {
        match self
            .decide_group_identity(
                state,
                user,
                identity,
                decision_id,
                Decision::ModelPair(decision),
            )
            .await?
        {
            DecisionOutcome::ModelPair(result) => Ok(result),
            _ => Err(invalid()),
        }
    }
}
