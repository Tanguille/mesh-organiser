//! Decision-bound downloads do not hold a database transaction during network I/O.

use sqlx::SqliteConnection;

use super::{Authorization, DbError, choice, invalid};

#[derive(Clone, Copy)]
pub enum Decision {
    Same,
    Separate,
    ModelPair(super::model_pair::ModelIdentityDecision),
}

pub enum DecisionOutcome {
    Same(super::GroupIdentityOutcome),
    Separate(super::GroupSeparationOutcome),
    ModelPair(super::model_pair::ModelIdentityOutcome),
}

/// The service holds the certified lease and verifies every byte before commit.
#[derive(Clone)]
pub struct LocalObservationStage {
    pub contents: Vec<crate::sync_transport::observations::BlobContent>,
    pub stage: crate::sync_transport::outbox::StageRecord,
}

pub enum IdentityTransfer {
    Complete(DecisionOutcome),
    Pending {
        delivery: Box<super::Delivery>,
        item: serde_json::Value,
        local_group_id: Option<i64>,
    },
}

pub async fn prepare_transfer(
    db: &super::DbContext,
    auth: &Authorization,
    decision_id: &str,
    decision: Decision,
) -> Result<IdentityTransfer, DbError> {
    let mut tx = db.begin().await?;
    if let Some(result) = retry_in(&mut tx, auth, decision_id, decision).await? {
        tx.commit().await?;

        return Ok(IdentityTransfer::Complete(result));
    }
    let row = choice(&mut tx, auth, decision_id).await?;
    let (delivery, _, snapshot) = match decision {
        Decision::ModelPair(_) => super::model_pair::pending(&mut tx, auth, &row).await?,
        _ => super::pending(&mut tx, auth, &row).await?,
    };
    let local_group_id = snapshot.has_models().then_some(row.local_group_id);
    let item = serde_json::from_str(&row.item).map_err(|_| invalid())?;
    tx.commit().await?;

    Ok(IdentityTransfer::Pending {
        delivery: Box::new(delivery),
        item,
        local_group_id,
    })
}

pub async fn retry_in(
    db: &mut SqliteConnection,
    auth: &Authorization,
    decision_id: &str,
    decision: Decision,
) -> Result<Option<DecisionOutcome>, DbError> {
    let row = choice(db, auth, decision_id).await?;
    let pair:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_model_identity_choices WHERE decision_id=? AND user_id=?)").bind(decision_id).bind(auth.user.id).fetch_one(&mut *db).await?;
    if pair != matches!(decision, Decision::ModelPair(_)) {
        return Err(invalid());
    }
    if row.writer_id.is_none() && row.counter.is_none() {
        return Ok(None);
    }
    if row.writer_id.is_none() || row.counter.is_none() {
        return Err(invalid());
    }

    finish_in(db, auth, decision_id, decision, &[], None)
        .await
        .map(Some)
}

pub async fn finish_in(
    db: &mut SqliteConnection,
    auth: &Authorization,
    decision_id: &str,
    decision: Decision,
    blobs: &[super::populated::ModelBlobBinding],
    local_source: Option<&LocalObservationStage>,
) -> Result<DecisionOutcome, DbError> {
    match decision {
        Decision::ModelPair(answer) => {
            super::model_pair::finish(db, auth, decision_id, answer, blobs, local_source)
                .await
                .map(DecisionOutcome::ModelPair)
        }
        Decision::Same => super::confirm_in(db, auth, decision_id, blobs, local_source)
            .await
            .map(DecisionOutcome::Same),
        Decision::Separate => {
            super::separation::separate_in(db, auth, decision_id, blobs, local_source)
                .await
                .map(DecisionOutcome::Separate)
        }
    }
}
