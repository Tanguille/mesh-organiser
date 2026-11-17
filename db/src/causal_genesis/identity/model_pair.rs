//! One explicit legacy pair. Identity consent does not select a file or metadata version.

use sqlx::SqliteConnection;

use super::{
    Authorization, DbContext, DbError, Delivery, GroupIdentityOutcome, choice, invalid,
    observations, populated,
};

#[derive(Debug)]
pub struct ModelIdentityChoice {
    pub decision_id: String,
    pub local_group_name: String,
    pub remote_group_name: String,
    pub local_model_id: i64,
    pub local_model_name: String,
    pub remote_model_name: String,
    /// Provisional Same eligibility for the selected group answer; files are verified before commit.
    pub same_model_supported: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupIdentityDecision {
    Same,
    Separate,
}
impl GroupIdentityDecision {
    const fn name(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Separate => "separate",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelIdentityDecision {
    Same,
    Separate,
}
impl ModelIdentityDecision {
    const fn name(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Separate => "separate",
        }
    }
}
#[derive(Debug)]
pub struct ModelIdentityOutcome {
    pub group: GroupIdentityOutcome,
    pub local_model_id: i64,
    pub remote_model_id: i64,
    pub remote_group_id: i64,
}

pub(super) async fn validate_pair(
    db: &mut SqliteConnection,
    user: &crate::model::user::User,
    snapshot: &populated::IdentitySnapshot,
    plan: &observations::ObservationPlan,
    model_id: i64,
) -> Result<bool, DbError> {
    plan.bundle.identity_group()?;
    let [local] = snapshot.models.as_slice() else {
        return Err(invalid());
    };
    let [remote] = plan.bundle.models.as_slice() else {
        return Err(invalid());
    };
    if local.id != model_id
        || local.birth_id.is_some()
        || local.deleted.is_some()
        || local.global_id != remote.entity().global_id
        || remote.entity().kind != crate::causal_envelope::EntityKind::Model
    {
        return Err(invalid());
    }
    let super::super::materialization::Materialization::Model(remote_data) =
        &remote.materialization
    else {
        return Err(invalid());
    };
    if remote_data.deleted.is_some() {
        return Err(invalid());
    }
    let history: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entity_births WHERE user_id=?1 AND entity_kind='model' AND (global_id=?2 OR birth_id=?3)) OR EXISTS(SELECT 1 FROM unknown_model_identity_tombstones WHERE user_id=?1 AND model_global_id=?2) OR EXISTS(SELECT 1 FROM membership_causal_events WHERE user_id=?1 AND model_global_id=?2) OR EXISTS(SELECT 1 FROM assignment_batch_journal WHERE user_id=?1) OR EXISTS(SELECT 1 FROM causal_events e,json_tree(e.envelope) r WHERE e.user_id=?1 AND (json_extract(e.envelope,'$.action.model_global_id')=?2 OR (json_extract(CASE WHEN r.type='object' THEN r.value ELSE '{}' END,'$.kind')='model' AND (json_extract(CASE WHEN r.type='object' THEN r.value ELSE '{}' END,'$.global_id')=?2 OR json_extract(CASE WHEN r.type='object' THEN r.value ELSE '{}' END,'$.birth_id')=?3))))")
        .bind(user.id).bind(&local.global_id).bind(&remote.entity().birth_id).fetch_one(db).await?;
    if history {
        return Err(invalid());
    }

    Ok(local.materialization() == remote.materialization)
}

pub async fn prepare(
    db: &DbContext,
    auth: &Authorization,
    decision_id: &str,
    model_id: i64,
    group_decision: GroupIdentityDecision,
) -> Result<ModelIdentityChoice, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let row = choice(&mut tx, auth, decision_id).await?;
    let (stored_model,prior):(i64,Option<String>)=sqlx::query_as("SELECT local_model_id,group_choice FROM sync_model_identity_choices WHERE decision_id=? AND user_id=?")
        .bind(decision_id).bind(auth.user.id).fetch_one(&mut *tx).await?;
    if stored_model != model_id {
        return Err(invalid());
    }
    let (_, plan, snapshot) = pending(&mut tx, auth, &row).await?;
    if let Some(prior) = prior {
        if prior != group_decision.name() {
            return Err(invalid());
        }
    } else {
        sqlx::query("UPDATE sync_model_identity_choices SET group_choice=? WHERE decision_id=? AND user_id=? AND group_choice IS NULL").bind(group_decision.name()).bind(decision_id).bind(auth.user.id).execute(&mut *tx).await?;
    }
    let supported = validate_pair(&mut tx, &auth.user, &snapshot, &plan, model_id).await?
        && group_decision == GroupIdentityDecision::Same;
    let remote: serde_json::Value =
        serde_json::from_str(&plan.bundle.models[0].materialization_json).map_err(|_| invalid())?;
    let group: serde_json::Value =
        serde_json::from_str(&plan.bundle.identity_group()?.materialization_json)
            .map_err(|_| invalid())?;
    tx.commit().await?;
    Ok(ModelIdentityChoice {
        decision_id: decision_id.to_owned(),
        local_group_name: snapshot.group.name,
        remote_group_name: group["name"].as_str().ok_or_else(invalid)?.into(),
        local_model_id: model_id,
        local_model_name: snapshot.models[0].name.clone(),
        remote_model_name: remote["name"].as_str().ok_or_else(invalid)?.into(),
        same_model_supported: supported,
    })
}

/// The coordinator offers only the bounded pair and never guesses from equal names or bytes.
pub async fn prepare_next(
    db: &DbContext,
    delivery: &Delivery,
    item: &serde_json::Value,
) -> Result<Option<super::GroupIdentityChoice>, DbError> {
    if item["kind"] != "observed_group" {
        return Ok(None);
    }
    let plan = observations::parse(item)?;
    let [remote] = plan.bundle.models.as_slice() else {
        return Ok(None);
    };
    let rows: Vec<(i64,i64)> = sqlx::query_as("SELECT g.group_id,m.model_id FROM models_group g JOIN models m ON m.model_group_id=g.group_id AND m.model_user_id=g.group_user_id WHERE g.group_user_id=? AND g.group_unique_global_id=? AND g.group_birth_id IS NULL AND m.model_unique_global_id=? AND m.model_birth_id IS NULL LIMIT 2")
        .bind(delivery.authorization.user.id).bind(&plan.bundle.group_entity().global_id).bind(&remote.entity().global_id).fetch_all(db).await?;
    let [(group, model)] = rows.as_slice() else {
        return Ok(None);
    };
    super::prepare_impl(db, delivery.clone(), item.clone(), *group, Some(*model))
        .await
        .map(Some)
}

pub(super) async fn pending(
    db: &mut SqliteConnection,
    auth: &Authorization,
    row: &super::ChoiceRecord,
) -> Result<
    (
        Delivery,
        observations::ObservationPlan,
        populated::IdentitySnapshot,
    ),
    DbError,
> {
    let position: super::Position = serde_json::from_str(&row.delivery).map_err(|_| invalid())?;
    let delivery = position.delivery(auth.clone());
    let item = serde_json::from_str(&row.item).map_err(|_| invalid())?;
    let plan = observations::parse(&item)?;
    let group = plan.bundle.identity_group()?;
    if plan.digest != delivery.digest {
        return Err(invalid());
    }
    observations::author(&plan, &delivery.issuer.writer_id)?;
    if delivery
        .validate(
            db,
            "observed_group",
            populated::transport::batch_id(&plan.bundle),
        )
        .await?
    {
        return Err(invalid());
    }
    let snapshot: populated::IdentitySnapshot =
        serde_json::from_str(&row.snapshot).map_err(|_| invalid())?;
    if snapshot != populated::IdentitySnapshot::read(db, &auth.user, row.local_group_id).await?
        || snapshot.group.birth_id.is_some()
        || snapshot.group.global_id != group.entity().global_id
    {
        return Err(invalid());
    }
    let [local] = snapshot.models.as_slice() else {
        return Err(invalid());
    };
    validate_pair(db, &auth.user, &snapshot, &plan, local.id).await?;
    super::separation::unmapped_history(db, auth.user.id, &snapshot.group.global_id).await?;
    Ok((delivery, plan, snapshot))
}

#[allow(clippy::too_many_lines)] // Both explicit answers, identity evidence, files and receipts commit together.
pub(super) async fn finish(
    db: &mut SqliteConnection,
    auth: &Authorization,
    id: &str,
    decision: ModelIdentityDecision,
    blobs: &[populated::ModelBlobBinding],
    source: Option<&super::LocalObservationStage>,
) -> Result<ModelIdentityOutcome, DbError> {
    let row = choice(db, auth, id).await?;
    let (local_model_id,previous,remote_model_id,group_choice,remote_group_id):(i64,Option<String>,Option<i64>,Option<String>,Option<i64>)=sqlx::query_as("SELECT local_model_id,choice,remote_model_id,group_choice,remote_group_id FROM sync_model_identity_choices WHERE decision_id=? AND user_id=?")
        .bind(id).bind(auth.user.id).fetch_one(&mut *db).await?;
    let group_choice = group_choice.ok_or_else(invalid)?;
    if group_choice == "separate" && decision == ModelIdentityDecision::Same {
        return Err(crate::sync_transport::failure(
            "model_membership_choice_required",
        ));
    }
    if let Some(previous) = previous {
        if previous != decision.name() {
            return Err(invalid());
        }
        let event = crate::causal_envelope::Dot {
            writer_id: row.writer_id.ok_or_else(invalid)?,
            counter: row.counter.ok_or_else(invalid)?.to_string(),
        };
        let outbox_id = row.local_outbox_id.ok_or_else(invalid)?;
        crate::sync_transport::outbox::require_retained(db, auth, &outbox_id, &event).await?;

        return Ok(ModelIdentityOutcome {
            group: GroupIdentityOutcome {
                group_id: row.local_group_id,
                event,
                already_present: true,
                outbox_id: Some(outbox_id),
            },
            local_model_id,
            remote_model_id: remote_model_id.ok_or_else(invalid)?,
            remote_group_id: remote_group_id.ok_or_else(invalid)?,
        });
    }
    let (delivery, plan, snapshot) = pending(db, auth, &row).await?;
    let source = source.ok_or_else(invalid)?;
    let local = &snapshot.models[0];
    let remote = plan.bundle.models[0].entity().clone();
    if decision == ModelIdentityDecision::Same
        && (local.materialization() != plan.bundle.models[0].materialization
            || source.contents != plan.contents())
    {
        return Err(crate::sync_transport::failure(
            "model_version_choice_required",
        ));
    }
    let (outcome, remote_group_id) = match decision {
        ModelIdentityDecision::Same => (
            populated::model_identity::enroll(
                db,
                auth,
                id,
                row.local_group_id,
                delivery,
                plan,
                blobs,
                source,
            )
            .await?,
            row.local_group_id,
        ),
        ModelIdentityDecision::Separate => {
            let fresh = crate::random_hex_32();
            let occupied:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models WHERE model_user_id=?1 AND model_unique_global_id=?2) OR EXISTS(SELECT 1 FROM entity_births WHERE user_id=?1 AND entity_kind='model' AND global_id=?2) OR EXISTS(SELECT 1 FROM unknown_model_identity_tombstones WHERE user_id=?1 AND model_global_id=?2)").bind(auth.user.id).bind(&fresh).fetch_one(&mut *db).await?;
            if occupied {
                return Err(invalid());
            }
            sqlx::query("UPDATE models SET model_unique_global_id=? WHERE model_id=? AND model_user_id=? AND model_incarnation=? AND model_birth_id IS NULL").bind(&fresh).bind(local.id).bind(auth.user.id).bind(&local.incarnation).execute(&mut *db).await?;
            let evidence:i64=sqlx::query_scalar("SELECT evidence_id FROM unknown_model_identity_evidence WHERE user_id=? AND model_global_id=? AND incarnation=? AND reason='rekey' ORDER BY evidence_id DESC LIMIT 1").bind(auth.user.id).bind(&local.global_id).bind(&local.incarnation).fetch_one(&mut *db).await?;
            // This one evidence record is certified. A later departure under the old id is not.
            sqlx::query("INSERT INTO model_identity_separation_evidence(decision_id,user_id,evidence_id,remote_global_id,remote_birth_id,local_global_id) VALUES(?,?,?,?,?,?)").bind(id).bind(auth.user.id).bind(evidence).bind(&remote.global_id).bind(&remote.birth_id).bind(&fresh).execute(&mut *db).await?;
            if group_choice == "separate" {
                let result = super::separation::separate_validated(
                    db,
                    auth,
                    id,
                    row.local_group_id,
                    delivery,
                    plan,
                    snapshot,
                    blobs,
                    Some(source),
                )
                .await?;
                (
                    GroupIdentityOutcome {
                        group_id: result.local_group_id,
                        event: result.event,
                        already_present: false,
                        outbox_id: result.outbox_id,
                    },
                    result.remote_group_id,
                )
            } else {
                (
                    populated::enroll_identity(
                        db,
                        auth,
                        id,
                        row.local_group_id,
                        delivery,
                        plan,
                        blobs,
                        source,
                    )
                    .await?,
                    row.local_group_id,
                )
            }
        }
    };
    let remote_model_id:i64=sqlx::query_scalar("SELECT local_id FROM causal_genesis_bindings WHERE user_id=? AND entity_kind='model' AND global_id=? AND birth_id=?").bind(auth.user.id).bind(&remote.global_id).bind(&remote.birth_id).fetch_one(&mut *db).await?;
    let (local_global,local_birth):(String,String)=sqlx::query_as("SELECT model_unique_global_id,model_birth_id FROM models WHERE model_id=? AND model_user_id=?").bind(local_model_id).bind(auth.user.id).fetch_one(&mut *db).await?;
    // Completion fields are persisted with the single immutable decision transition.
    sqlx::query("UPDATE sync_model_identity_choices SET choice=?,remote_model_id=?,local_global_id=?,local_birth_id=?,remote_group_id=? WHERE decision_id=? AND user_id=? AND choice IS NULL")
        .bind(decision.name()).bind(remote_model_id).bind(local_global).bind(local_birth).bind(remote_group_id).bind(id).bind(auth.user.id).execute(db).await?;
    Ok(ModelIdentityOutcome {
        group: outcome,
        local_model_id,
        remote_model_id,
        remote_group_id,
    })
}
