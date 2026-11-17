//! Supplemental batches retain their own membership boundary and unknown ancestry.

use sqlx::SqliteConnection;

use crate::causal_genesis::identity::{self, GroupIdentityOutcome, IndependentObservation};

use super::{
    DbError, ObservedPopulatedGroupBundle, PopulatedGroupInstallation, invalid, persistence,
    transport,
};

pub(super) fn observation(
    bundle: &ObservedPopulatedGroupBundle,
) -> Result<IndependentObservation, DbError> {
    let link = bundle.identity_link.as_ref().ok_or_else(invalid)?;
    if bundle.labels.is_some() || bundle.project.is_some() || bundle.models.is_empty() {
        return Err(invalid());
    }
    Ok(IndependentObservation {
        observation_id: link.observation_id.clone(),
        baseline: link.baseline.clone(),
        root: bundle.group.wire.clone(),
        source_identity: bundle.source_identity().clone(),
        materialization: bundle.group.materialization_json.clone(),
        proof: bundle
            .group
            .observation_evidence
            .clone()
            .ok_or_else(invalid)?,
        batch_id: Some(bundle.batch_id.clone()),
    })
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Both snapshots, verified bytes, outbox and incoming receipt share this transaction.
pub(in crate::causal_genesis) async fn enroll_identity(
    db: &mut SqliteConnection,
    auth: &crate::sync_transport::pull::Authorization,
    decision_id: &str,
    group_id: i64,
    delivery: crate::sync_transport::pull::Delivery,
    mut plan: crate::sync_transport::observations::ObservationPlan,
    blobs: &[super::ModelBlobBinding],
    source: &identity::LocalObservationStage,
) -> Result<GroupIdentityOutcome, DbError> {
    let remote = plan.bundle.identity_group()?;
    let entity = remote.entity().clone();
    let baseline = remote.event().clone();
    identity::separation::unmapped_history(db, auth.user.id, &entity.global_id).await?;
    let capture = super::observe_in(db, &auth.user, &auth.identity, group_id).await?;
    let local = persistence::enroll_boundary(
        db,
        &auth.user,
        &auth.identity,
        &capture,
        Some(&source.contents),
        entity.birth_id.as_deref(),
        Some(transport::IdentityLink {
            observation_id: decision_id.to_owned(),
            baseline,
            model: None,
        }),
    )
    .await?;
    // The primary remote root keeps its own child dependencies. The local root
    // is stored only after that binding exists, and claims only local children.
    super::bind_existing_identity(
        db,
        &auth.user,
        &auth.identity,
        auth.issuer_origin(&delivery.issuer),
        &mut plan.bundle,
        group_id,
        blobs,
    )
    .await?;
    identity::insert(db, &auth.user, &observation(&local)?, true).await?;
    crate::causal_group_lifecycle::project_identity_observation(db, &auth.user).await?;
    crate::sync_transport::observations::record_source_export(
        db,
        &auth.user,
        &local,
        Some(&source.contents),
    )
    .await?;
    let (outbox_id, _) = crate::sync_transport::outbox::queue_enrolled_observation_in(
        db,
        auth,
        &local,
        &source.contents,
        &source.stage,
    )
    .await?;
    delivery
        .commit(db, "observed_group", transport::batch_id(&plan.bundle))
        .await?;
    let event = local.group.event().clone();
    sqlx::query("UPDATE sync_group_identity_choices SET writer_id=?,counter=?,local_outbox_id=? WHERE decision_id=? AND user_id=? AND writer_id IS NULL")
        .bind(&event.writer_id).bind(super::super::counter(&event)?).bind(&outbox_id).bind(decision_id).bind(auth.user.id).execute(db).await?;
    Ok(GroupIdentityOutcome {
        group_id,
        event,
        already_present: false,
        outbox_id: Some(outbox_id),
    })
}

pub(super) async fn install(
    db: &mut SqliteConnection,
    user: &super::User,
    identity: &super::SyncIdentity,
    bundle: &ObservedPopulatedGroupBundle,
    blobs: &[super::ModelBlobBinding],
) -> Result<PopulatedGroupInstallation, DbError> {
    let observation = observation(bundle)?;
    observation.validate()?;
    let prior: Option<String> = sqlx::query_scalar(
        "SELECT batch_id FROM group_identity_observations WHERE user_id=? AND observation_id=?",
    )
    .bind(user.id)
    .bind(&observation.observation_id)
    .fetch_optional(&mut *db)
    .await?;
    if prior.is_some() {
        return Err(invalid());
    }
    let guard = crate::causal_group_lifecycle::prepare_identity_extension(
        db,
        user,
        bundle.group.entity(),
        &observation.baseline,
    )
    .await?;
    if bundle
        .identity_link
        .as_ref()
        .is_some_and(|link| link.model.is_some())
    {
        return super::model_identity::install(db, user, identity, bundle, blobs, guard).await;
    }
    bundle.novel_models(db, user).await?;
    let blob_ids = persistence::validate_blobs(db, bundle, blobs).await?;
    persistence::write_batch(db, user, identity, bundle, "installed").await?;
    for member in &bundle.models {
        crate::causal_store::store_checked_receipt(db, user.id, &member.envelope, &member.wire)
            .await?;
    }
    let models = persistence::install_member_rows(
        db,
        user,
        identity,
        bundle,
        guard.materialized_group_id(),
        blob_ids,
    )
    .await?;
    identity::insert(db, user, &observation, false).await?;
    let group_id =
        crate::causal_group_lifecycle::finish_identity_extension(db, user, guard).await?;
    Ok(PopulatedGroupInstallation {
        group_id,
        models,
        already_present: false,
    })
}
