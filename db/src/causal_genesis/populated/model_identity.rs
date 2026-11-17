//! A second model observation is evidence of an explicit match, never a causal parent.

use sqlx::SqliteConnection;

use crate::causal_genesis::{
    identity::{self, GroupIdentityOutcome, IndependentRoot},
    materialization, storage,
};
use crate::{
    DbError,
    causal_envelope::{CausalEnvelope, Dependency},
    model::user::User,
    user_db::SyncIdentity,
};

use super::{
    ModelBlobBinding, ObservedPopulatedGroupBundle, PopulatedGroupInstallation, invalid,
    persistence,
};

pub(super) fn validate_link(bundle: &ObservedPopulatedGroupBundle) -> Result<(), DbError> {
    let Some(link) = bundle
        .identity_link
        .as_ref()
        .and_then(|link| link.model.as_ref())
    else {
        return Ok(());
    };
    let [model] = bundle.models.as_slice() else {
        return Err(invalid());
    };
    crate::causal_envelope::counter(&link.event).map_err(|_| invalid())?;
    let proof: serde_json::Value =
        serde_json::from_str(model.observation_evidence.as_deref().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
    if link.entity != *model.entity()
        || link.event == *model.event()
        || !proof["original"]["birth_id"].is_null()
    {
        return Err(invalid());
    }

    Ok(())
}

/// A model match is scoped to the original authenticated membership boundary.
/// Two separately valid roots do not establish that they were observed together.
async fn matching_baseline(
    db: &mut SqliteConnection,
    user: &User,
    bundle: &ObservedPopulatedGroupBundle,
) -> Result<(), DbError> {
    let link = bundle.identity_link.as_ref().ok_or_else(invalid)?;
    let model = link.model.as_ref().ok_or_else(invalid)?;
    let group = Dependency {
        entity: bundle.group.entity().clone(),
        event: link.baseline.clone(),
    };
    let (model_batch, model_root, model_proof) = baseline(db, user, model).await?;
    let (group_batch, group_root, group_proof) = baseline(db, user, &group).await?;
    let destination: crate::causal_envelope::Entity =
        serde_json::from_value(model_proof["initial_group"].clone()).map_err(|_| invalid())?;
    if model_batch != group_batch
        || model_proof["batch_id"] != model_batch
        || group_proof["batch_id"] != group_batch
        || destination != group.entity
        || !group_root.dependencies.contains(model)
        || model_root.entity != model.entity
    {
        return Err(invalid());
    }

    Ok(())
}

async fn baseline(
    db: &mut SqliteConnection,
    user: &User,
    reference: &Dependency,
) -> Result<(String, CausalEnvelope, serde_json::Value), DbError> {
    let (batch, wire, proof): (String, String, String) = sqlx::query_as("SELECT m.batch_id,e.envelope,b.observation_evidence FROM causal_genesis_bindings b JOIN causal_observation_batch_members m USING(user_id,entity_kind,birth_id) JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.entity_kind=? AND b.global_id=? AND b.birth_id=? AND b.writer_id=? AND b.counter=? AND b.capability='group_lifecycle_v1' AND b.initialization_kind='observation'")
        .bind(user.id).bind(super::super::kind(&reference.entity)).bind(&reference.entity.global_id).bind(&reference.entity.birth_id).bind(&reference.event.writer_id).bind(super::super::counter(&reference.event)?).fetch_optional(db).await?.ok_or_else(invalid)?;
    let root = crate::causal_envelope::parse_causal_envelope(&wire).map_err(|_| invalid())?;
    if root.entity != reference.entity || root.event != reference.event {
        return Err(invalid());
    }
    Ok((
        batch,
        root,
        serde_json::from_str(&proof).map_err(|_| invalid())?,
    ))
}

async fn insert(
    db: &mut SqliteConnection,
    user: &User,
    bundle: &ObservedPopulatedGroupBundle,
    local: bool,
) -> Result<i64, DbError> {
    validate_link(bundle)?;
    matching_baseline(db, user, bundle).await?;
    let link = bundle
        .identity_link
        .as_ref()
        .and_then(|link| link.model.as_ref())
        .ok_or_else(invalid)?;
    let model = bundle.models.first().ok_or_else(invalid)?;
    let row:Option<(i64,String,String)>=sqlx::query_as("SELECT b.local_id,b.materialization,b.local_guard FROM causal_genesis_bindings b JOIN models m ON m.model_id=b.local_id AND m.model_user_id=b.user_id JOIN group_lifecycle_scopes s USING(user_id,entity_kind,birth_id) WHERE b.user_id=? AND b.entity_kind='model' AND b.global_id=? AND b.birth_id=? AND b.writer_id=? AND b.counter=? AND m.model_unique_global_id=b.global_id AND m.model_birth_id=b.birth_id AND m.model_incarnation=json_extract(b.local_guard,'$.incarnation') AND m.model_blob_id=json_extract(b.local_guard,'$.blob_id') AND m.model_deleted IS NULL AND s.paused=0")
        .bind(user.id).bind(&link.entity.global_id).bind(&link.entity.birth_id).bind(&link.event.writer_id).bind(crate::causal_envelope::counter(&link.event).map_err(|_|invalid())?).fetch_optional(&mut *db).await?;
    let (id, materialization, _guard) = row.ok_or_else(invalid)?;
    if materialization != model.materialization_json {
        return Err(invalid());
    }
    let (count,bytes):(i64,i64)=sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(length(CAST(root AS BLOB))+length(CAST(materialization AS BLOB))+length(CAST(proof AS BLOB))),0) FROM model_identity_observations WHERE user_id=?")
        .bind(user.id).fetch_one(&mut *db).await?;
    let added = model.wire.len()
        + model.materialization_json.len()
        + model.observation_evidence.as_ref().map_or(0, String::len);
    if usize::try_from(count).map_err(|_| invalid())? >= crate::causal_store::MAX_OWNER_EVENTS
        || usize::try_from(bytes)
            .map_err(|_| invalid())?
            .saturating_add(added)
            > crate::causal_store::MAX_OWNER_BYTES
    {
        return Err(invalid());
    }
    crate::causal_store::check_quota(
        db,
        user.id,
        model.wire.len()
            + model.materialization_json.len()
            + model.observation_evidence.as_ref().map_or(0, String::len),
        0,
    )
    .await?;
    if local {
        sqlx::query(
            "INSERT INTO causal_events(user_id,writer_id,counter,envelope) VALUES(?,?,?,?)",
        )
        .bind(user.id)
        .bind(&model.event().writer_id)
        .bind(super::super::counter(model.event())?)
        .bind(&model.wire)
        .execute(&mut *db)
        .await?;
    } else {
        crate::causal_store::store_checked_receipt(db, user.id, model.envelope(), &model.wire)
            .await?;
    }
    sqlx::query("INSERT INTO model_identity_observations(user_id,batch_id,birth_id,writer_id,counter,root,materialization,proof) VALUES(?,?,?,?,?,?,?,?)")
        .bind(user.id).bind(&bundle.batch_id).bind(&model.entity().birth_id).bind(&model.event().writer_id).bind(super::super::counter(model.event())?).bind(&model.wire).bind(&model.materialization_json).bind(&model.observation_evidence).execute(db).await?;
    Ok(id)
}

#[allow(clippy::too_many_arguments)] // Both independent boundaries, bytes and receipt share one transaction.
pub(in crate::causal_genesis) async fn enroll(
    db: &mut SqliteConnection,
    auth: &crate::sync_transport::pull::Authorization,
    decision_id: &str,
    group_id: i64,
    delivery: crate::sync_transport::pull::Delivery,
    mut plan: crate::sync_transport::observations::ObservationPlan,
    blobs: &[ModelBlobBinding],
    source: &identity::LocalObservationStage,
) -> Result<GroupIdentityOutcome, DbError> {
    let capture = super::observe_in(db, &auth.user, &auth.identity, group_id).await?;
    let remote = plan.bundle.models.first().ok_or_else(invalid)?;
    let link = super::transport::IdentityLink {
        observation_id: decision_id.to_owned(),
        baseline: plan.bundle.group.event().clone(),
        model: Some(Dependency {
            entity: remote.entity().clone(),
            event: remote.event().clone(),
        }),
    };
    let local = persistence::enroll_boundary(
        db,
        &auth.user,
        &auth.identity,
        &capture,
        Some(&source.contents),
        plan.bundle.group.entity().birth_id.as_deref(),
        Some(link),
    )
    .await?;
    persistence::validate_blobs(db, &plan.bundle, blobs).await?;
    plan.bundle.verified_origin = Some(auth.issuer_origin(&delivery.issuer));
    persistence::write_batch(db, &auth.user, &auth.identity, &plan.bundle, "installed").await?;
    for member in plan
        .bundle
        .models
        .iter()
        .chain(std::iter::once(&plan.bundle.group))
    {
        crate::causal_store::store_checked_receipt(
            db,
            auth.user.id,
            member.envelope(),
            &member.wire,
        )
        .await?;
    }
    let remote = &plan.bundle.models[0];
    let model_id = capture.models[0].id;
    let guard = persistence::observed_guard(db, &auth.user, remote.entity(), model_id).await?;
    storage::write_binding(
        db,
        &auth.user,
        &auth.identity,
        remote,
        model_id,
        &guard,
        "installed",
    )
    .await?;
    let guard =
        materialization::current_guard(db, &auth.user, plan.bundle.group.entity(), group_id)
            .await?;
    storage::write_binding(
        db,
        &auth.user,
        &auth.identity,
        &plan.bundle.group,
        group_id,
        &guard,
        "installed",
    )
    .await?;
    persistence::write_members(db, &auth.user, &plan.bundle).await?;
    sqlx::query("INSERT INTO model_identity_adoptions(user_id,birth_id,model_global_id,model_incarnation) VALUES(?,?,?,?)")
        .bind(auth.user.id).bind(&remote.entity().birth_id).bind(&capture.models[0].global_id).bind(&capture.models[0].incarnation).execute(&mut *db).await?;
    insert(db, &auth.user, &local, true).await?;
    identity::insert(db, &auth.user, &super::identity::observation(&local)?, true).await?;
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
        .commit(db, "observed_group", &plan.bundle.batch_id)
        .await?;
    let event = local.group.event().clone();
    sqlx::query("UPDATE sync_group_identity_choices SET writer_id=?,counter=?,local_outbox_id=? WHERE decision_id=? AND user_id=? AND writer_id IS NULL").bind(&event.writer_id).bind(super::super::counter(&event)?).bind(&outbox_id).bind(decision_id).bind(auth.user.id).execute(db).await?;
    Ok(GroupIdentityOutcome {
        group_id,
        event,
        already_present: false,
        outbox_id: Some(outbox_id),
    })
}

pub(super) async fn install(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    bundle: &ObservedPopulatedGroupBundle,
    blobs: &[ModelBlobBinding],
    guard: crate::causal_group_lifecycle::IdentityExtensionGuard,
) -> Result<PopulatedGroupInstallation, DbError> {
    persistence::validate_blobs(db, bundle, blobs).await?;
    persistence::write_batch(db, user, identity, bundle, "installed").await?;
    let model_id = insert(db, user, bundle, false).await?;
    let matches:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models WHERE model_id=? AND model_user_id=? AND model_blob_id=?)").bind(model_id).bind(user.id).bind(blobs[0].local_blob_id).fetch_one(&mut *db).await?;
    if !matches {
        return Err(invalid());
    }
    identity::insert(db, user, &super::identity::observation(bundle)?, false).await?;
    let group_id =
        crate::causal_group_lifecycle::finish_identity_extension(db, user, guard).await?;
    Ok(PopulatedGroupInstallation {
        group_id,
        models: vec![(bundle.models[0].entity().clone(), model_id)],
        already_present: false,
    })
}

pub(in crate::causal_genesis) async fn roots(
    db: &mut SqliteConnection,
    user: &User,
    entity: &crate::causal_envelope::Entity,
) -> Result<Vec<IndependentRoot>, DbError> {
    let rows:Vec<(String,String,String,String,String,String)>=sqlx::query_as("SELECT m.root,m.materialization,m.proof,m.batch_id,b.source_instance_id,b.source_account_id FROM model_identity_observations m JOIN causal_observation_batches b USING(user_id,batch_id) WHERE m.user_id=? AND m.birth_id=? ORDER BY m.writer_id,m.counter LIMIT 4097").bind(user.id).bind(&entity.birth_id).fetch_all(db).await?;
    if rows.len() > crate::causal_store::MAX_OWNER_EVENTS {
        return Err(invalid());
    }
    rows.into_iter()
        .map(
            |(wire, materialization, proof, batch_id, instance_id, account_id)| {
                let root: CausalEnvelope =
                    crate::causal_envelope::parse_causal_envelope(&wire).map_err(|_| invalid())?;
                let (name, destination) = super::validate(
                    &root,
                    &materialization,
                    &proof,
                    &SyncIdentity {
                        instance_id,
                        account_id,
                    },
                )?;
                if root.entity != *entity || name.is_some() || destination.is_none() {
                    return Err(invalid());
                }
                Ok(IndependentRoot {
                    root,
                    name: None,
                    destination,
                    batch_id: Some(batch_id),
                })
            },
        )
        .collect()
}
