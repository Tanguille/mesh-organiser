use sqlx::SqliteConnection;

use crate::{causal_envelope::Dependency, sync_peer::validate_local, util::sha256_hex};

use super::super::{checks, counter, encode, kind, materialization::Guard, storage};
use super::{
    DbContext, DbError, Entity, EntityKind, GenesisBundle, Materialization, ModelBlobBinding,
    ObservedPopulatedGroupBundle, PopulatedGroupInstallation, PopulatedGroupObservation, Proof,
    SyncIdentity, User, capture, invalid, labels, materialization, project,
};

async fn make_bundle(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    entity: Entity,
    value: Materialization,
    proof: Proof,
) -> Result<GenesisBundle, DbError> {
    let materialization_json = encode(&value)?;
    let proof = encode(&proof)?;
    let (writer_id, counter) = crate::causal_clock::next(db, user.id).await?;
    let decoded: Proof = serde_json::from_str(&proof).map_err(|_| invalid())?;
    let envelope = super::CausalEnvelope {
        version: 2,
        entity,
        event: crate::causal_envelope::Dot {
            writer_id,
            counter: counter.to_string(),
        },
        parents: vec![],
        dependencies: decoded.members,
        action: crate::causal_envelope::Action::LegacyObserved {
            observation_digest: sha256_hex(proof.as_bytes()),
        },
    };
    let wire =
        crate::causal_envelope::canonical_causal_envelope(&envelope).map_err(|_| invalid())?;
    Ok(GenesisBundle {
        envelope,
        wire,
        source_identity: identity.clone(),
        materialization: value,
        materialization_json,
        capability: "group_lifecycle_v1".into(),
        observation_evidence: Some(proof),
    })
}

fn proof(
    capture: &PopulatedGroupObservation,
    original: serde_json::Value,
    value: &Materialization,
    initial_group: Option<Entity>,
    members: Vec<Dependency>,
    label_digest: Option<String>,
) -> Result<Proof, DbError> {
    Ok(Proof {
        kind: if initial_group.is_some() {
            "populated_model"
        } else {
            "populated_group"
        }
        .into(),
        batch_id: capture.batch_id.clone(),
        source_instance_id: capture.identity.instance_id.clone(),
        source_account_id: capture.identity.account_id.clone(),
        original,
        initial_group,
        members,
        materialization_digest: sha256_hex(encode(value)?.as_bytes()),
        history: "unknown_before_observation".into(),
        label_snapshot_digest: label_digest,
        project_snapshot_digest: capture
            .project
            .as_ref()
            .map(encode)
            .transpose()?
            .as_deref()
            .map(|value| sha256_hex(value.as_bytes())),
    })
}

async fn permit_models(
    db: &mut SqliteConnection,
    user: &User,
    capture: &PopulatedGroupObservation,
    model_match: Option<&Dependency>,
) -> Result<Vec<Entity>, DbError> {
    let mut entities = Vec::new();
    for model in &capture.models {
        let mut entity = model.entity();
        if let Some(selected) = model_match {
            if capture.models.len() != 1
                || entity.global_id != selected.entity.global_id
                || entity.birth_id.is_some()
            {
                return Err(invalid());
            }
            entity.birth_id = selected.entity.birth_id.clone();
        } else {
            entity.birth_id = Some(entity.birth_id.unwrap_or_else(crate::random_hex_32));
        }
        sqlx::query("INSERT INTO model_birth_adoption_permits(model_id,user_id,account_id,instance_id,global_id,incarnation,original_birth_id,birth_id,group_id,observation_revision,label_count,deleted_at) VALUES(?,?,?,?,?,?,?,?,?,?,(SELECT COUNT(*) FROM models_labels WHERE model_id=?),?)")
            .bind(model.id).bind(user.id).bind(&capture.identity.account_id).bind(&capture.identity.instance_id).bind(&model.global_id).bind(&model.incarnation).bind(&model.birth_id).bind(&entity.birth_id).bind(capture.group_id).bind(model.epoch).bind(model.id).bind(&model.deleted).execute(&mut *db).await?;
        entities.push(entity);
    }

    Ok(entities)
}

async fn adopt_group(
    db: &mut SqliteConnection,
    user: &User,
    capture: &PopulatedGroupObservation,
    selected_birth: Option<&str>,
) -> Result<Entity, DbError> {
    let entity = Entity {
        kind: EntityKind::Group,
        global_id: capture.group.global_id.clone(),
        birth_id: Some(
            capture
                .group
                .birth_id
                .clone()
                .or_else(|| selected_birth.map(str::to_owned))
                .unwrap_or_else(crate::random_hex_32),
        ),
    };
    if capture.group.birth_id.is_none() {
        sqlx::query("INSERT INTO group_birth_adoption_permits(group_id,user_id,account_id,instance_id,global_id,birth_id,observation_revision,head,member_count,resource_id,resource_incarnation,resource_revision) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind(capture.group_id).bind(user.id).bind(&capture.identity.account_id).bind(&capture.identity.instance_id).bind(&entity.global_id).bind(&entity.birth_id).bind(capture.group.epoch).bind(capture.group.head).bind(i64::try_from(capture.models.len()).map_err(|_|invalid())?).bind(capture.project.as_ref().map(|captured_project|captured_project.id)).bind(capture.project.as_ref().map(|captured_project|&captured_project.incarnation)).bind(capture.project.as_ref().map(|captured_project|captured_project.revision)).execute(&mut *db).await?;
        sqlx::query("UPDATE models_group SET group_birth_id=? WHERE group_id=? AND group_user_id=? AND group_birth_id IS NULL")
            .bind(&entity.birth_id).bind(capture.group_id).bind(user.id).execute(db).await?;
    }

    Ok(entity)
}

#[allow(clippy::too_many_lines)] // One transaction validates and adopts the complete observed boundary.
pub(super) async fn enroll(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    capture: &PopulatedGroupObservation,
    transport_contents: Option<&[super::transport::BlobContent]>,
) -> Result<ObservedPopulatedGroupBundle, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let bundle = enroll_in(&mut tx, user, identity, capture, transport_contents).await?;
    tx.commit().await?;

    Ok(bundle)
}

#[allow(clippy::too_many_lines)] // One atomic enrollment preserves all captured identity and metadata guards.
pub(super) async fn enroll_in(
    tx: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    capture: &PopulatedGroupObservation,
    transport_contents: Option<&[super::transport::BlobContent]>,
) -> Result<ObservedPopulatedGroupBundle, DbError> {
    enroll_boundary(tx, user, identity, capture, transport_contents, None, None).await
}

#[allow(clippy::too_many_lines)] // Validate and persist the complete captured boundary in one transaction.
pub(super) async fn enroll_boundary(
    tx: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    capture: &PopulatedGroupObservation,
    transport_contents: Option<&[super::transport::BlobContent]>,
    selected_birth: Option<&str>,
    identity_link: Option<super::transport::IdentityLink>,
) -> Result<ObservedPopulatedGroupBundle, DbError> {
    validate_local(tx, user, identity).await?;
    if capture.owner != user.id || capture.identity != *identity {
        return Err(invalid());
    }
    if let Some(bundle) =
        load_retry(tx, user, identity, &capture.batch_id, capture.models.len()).await?
    {
        validate_transport(&bundle, transport_contents)?;
        crate::sync_transport::observations::record_source_export(
            tx,
            user,
            &bundle,
            transport_contents,
        )
        .await?;

        return Ok(bundle);
    }
    let (group, models) = capture::read(tx, user, capture.group_id).await?;
    if group != capture.group
        || models != capture.models
        || labels::capture(tx, user, capture.group_id).await? != capture.labels
        || project::capture(tx, user, group.resource_id).await? != capture.project
    {
        return Err(invalid());
    }
    let entities = permit_models(
        tx,
        user,
        capture,
        identity_link.as_ref().and_then(|link| link.model.as_ref()),
    )
    .await?;
    let group = adopt_group(tx, user, capture, selected_birth).await?;
    let labels = labels::bind_models(&capture.labels, &entities)?;
    let label_snapshot = labels.as_ref().map(encode).transpose()?;
    let label_digest = label_snapshot
        .as_deref()
        .map(|value| sha256_hex(value.as_bytes()));
    let mut models = Vec::new();
    for (model, entity) in capture.models.iter().zip(entities) {
        if model.birth_id.is_none() {
            sqlx::query("UPDATE models SET model_birth_id=? WHERE model_id=? AND model_user_id=? AND model_birth_id IS NULL")
                .bind(&entity.birth_id).bind(model.id).bind(user.id).execute(&mut *tx).await?;
        }
        sqlx::query("DELETE FROM model_birth_adoption_permits WHERE model_id=?")
            .bind(model.id)
            .execute(&mut *tx)
            .await?;
        let value = model.materialization();
        let evidence = proof(
            capture,
            serde_json::to_value(model).map_err(|_| invalid())?,
            &value,
            Some(group.clone()),
            vec![],
            label_digest.clone(),
        )?;
        models.push(make_bundle(tx, user, identity, entity, value, evidence).await?);
    }
    let value = Materialization::Group(materialization::GroupSnapshot {
        name: capture.group.name.clone(),
        created: capture.group.created.clone(),
        last_modified: capture.group.last_modified.clone(),
    });
    let mut members: Vec<_> = models
        .iter()
        .map(|model| Dependency {
            entity: model.entity().clone(),
            event: model.event().clone(),
        })
        .collect();
    members.sort_by(|left, right| {
        (&left.event.writer_id, &left.event.counter)
            .cmp(&(&right.event.writer_id, &right.event.counter))
    });
    let evidence = proof(
        capture,
        serde_json::to_value(&capture.group).map_err(|_| invalid())?,
        &value,
        None,
        members,
        label_digest,
    )?;
    let group = make_bundle(tx, user, identity, group, value, evidence).await?;
    let mut bundle = assemble(
        capture.batch_id.clone(),
        group,
        models,
        labels,
        capture.project.clone(),
    )?;
    bundle.identity_link = identity_link;
    // Validate exact network serialization before committing any adopted identity,
    // root, binding or clock. The in-process observation API has separate bounds.
    validate_transport(&bundle, transport_contents)?;
    write_batch(tx, user, identity, &bundle, "local").await?;
    if let Some(project) = &bundle.project {
        project::enroll(tx, user, identity, project).await?;
    }
    if let Some(labels) = &bundle.labels {
        labels::enroll(tx, user, identity, labels).await?;
    }
    for (model, data) in bundle.models.iter().zip(&capture.models) {
        if bundle
            .identity_link
            .as_ref()
            .is_some_and(|link| link.model.is_some())
        {
            continue;
        }
        write_local(tx, user, identity, model, data.id).await?;
        sqlx::query("INSERT INTO model_identity_adoptions(user_id,birth_id,model_global_id,model_incarnation) VALUES(?,?,?,?)")
            .bind(user.id).bind(&model.entity().birth_id).bind(&data.global_id).bind(&data.incarnation).execute(&mut *tx).await?;
    }
    if bundle.identity_link.is_none() {
        write_local(tx, user, identity, &bundle.group, capture.group_id).await?;
    }
    write_members(tx, user, &bundle).await?;
    if bundle.identity_link.is_none() {
        crate::sync_transport::observations::record_source_export(
            tx,
            user,
            &bundle,
            transport_contents,
        )
        .await?;
    }

    Ok(bundle)
}

fn validate_transport(
    bundle: &ObservedPopulatedGroupBundle,
    contents: Option<&[super::transport::BlobContent]>,
) -> Result<(), DbError> {
    if let Some(contents) = contents {
        crate::sync_transport::observations::encode(bundle, contents.to_vec())?;
    }

    Ok(())
}

fn assemble(
    batch_id: String,
    group: GenesisBundle,
    models: Vec<GenesisBundle>,
    labels: Option<labels::Snapshot>,
    project: Option<project::Snapshot>,
) -> Result<ObservedPopulatedGroupBundle, DbError> {
    let references: Vec<_> = models
        .iter()
        .chain(std::iter::once(&group))
        .map(|bundle| Dependency {
            entity: bundle.entity().clone(),
            event: bundle.event().clone(),
        })
        .collect();
    let manifest = encode(&references)?;
    let model_envelopes = models
        .iter()
        .map(|bundle| bundle.envelope.clone())
        .collect();
    let label_snapshot = labels.as_ref().map(encode).transpose()?;
    let project_snapshot = project.as_ref().map(encode).transpose()?;
    Ok(ObservedPopulatedGroupBundle {
        group,
        models,
        model_envelopes,
        batch_id,
        manifest,
        labels,
        label_snapshot,
        project,
        project_snapshot,
        verified_origin: None,
        identity_link: None,
    })
}

pub(super) async fn write_batch(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    bundle: &ObservedPopulatedGroupBundle,
    origin: &str,
) -> Result<(), DbError> {
    sqlx::query("INSERT INTO causal_observation_batches(user_id,batch_id,local_instance_id,local_account_id,source_instance_id,source_account_id,manifest,origin,label_snapshot,project_snapshot,verified_source_instance,verified_source_account,identity_link) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(user.id).bind(&bundle.batch_id).bind(&identity.instance_id).bind(&identity.account_id).bind(&bundle.source_identity().instance_id).bind(&bundle.source_identity().account_id).bind(&bundle.manifest).bind(origin).bind(&bundle.label_snapshot).bind(&bundle.project_snapshot).bind(bundle.verified_origin.as_ref().map(|source|&source.instance_id)).bind(bundle.verified_origin.as_ref().map(|source|&source.account_id)).bind(bundle.identity_link.as_ref().map(encode).transpose()?).execute(db).await?;
    Ok(())
}
pub(super) async fn write_members(
    db: &mut SqliteConnection,
    user: &User,
    bundle: &ObservedPopulatedGroupBundle,
) -> Result<(), DbError> {
    for member in bundle
        .models
        .iter()
        .filter(|_| {
            bundle
                .identity_link
                .as_ref()
                .is_none_or(|link| link.model.is_none())
        })
        .chain(bundle.identity_link.is_none().then_some(&bundle.group))
    {
        sqlx::query("INSERT INTO causal_observation_batch_members(user_id,batch_id,entity_kind,birth_id) VALUES(?,?,?,?)")
            .bind(user.id).bind(&bundle.batch_id).bind(kind(member.entity())).bind(&member.entity().birth_id).execute(&mut *db).await?;
    }

    Ok(())
}

async fn write_local(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    bundle: &GenesisBundle,
    id: i64,
) -> Result<(), DbError> {
    sqlx::query("INSERT INTO causal_events(user_id,writer_id,counter,envelope) VALUES(?,?,?,?)")
        .bind(user.id)
        .bind(&bundle.event().writer_id)
        .bind(counter(bundle.event())?)
        .bind(&bundle.wire)
        .execute(&mut *db)
        .await?;
    let guard = observed_guard(db, user, bundle.entity(), id).await?;
    storage::write_binding(db, user, identity, bundle, id, &guard, "local").await
}

pub(super) async fn observed_guard(
    db: &mut SqliteConnection,
    user: &User,
    entity: &Entity,
    id: i64,
) -> Result<Guard, DbError> {
    let mut guard = materialization::current_guard(db, user, entity, id).await?;
    if let Guard::Model {
        observation_revision,
        ..
    } = &mut guard
    {
        *observation_revision = Some(
            sqlx::query_scalar("SELECT revision FROM model_observation_versions WHERE model_id=?")
                .bind(id)
                .fetch_one(db)
                .await?,
        );
    }

    Ok(guard)
}

pub(super) async fn load_retry(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    batch_id: &str,
    model_count: usize,
) -> Result<Option<ObservedPopulatedGroupBundle>, DbError> {
    let prior:Option<(String,Option<String>,Option<String>)>=sqlx::query_as("SELECT manifest,label_snapshot,project_snapshot FROM causal_observation_batches WHERE user_id=? AND batch_id=? AND local_instance_id=? AND local_account_id=? AND source_instance_id=? AND source_account_id=? AND origin='local'")
        .bind(user.id).bind(batch_id).bind(&identity.instance_id).bind(&identity.account_id).bind(&identity.instance_id).bind(&identity.account_id).fetch_optional(&mut *db).await?;
    let Some((manifest, label_snapshot, project_snapshot)) = prior else {
        return Ok(None);
    };
    let project: Option<project::Snapshot> = project_snapshot
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| invalid())?;
    if let Some(project) = &project
        && !project::valid(db, user, identity, project).await?
    {
        return Err(invalid());
    }
    let labels: Option<labels::Snapshot> = label_snapshot
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| invalid())?;
    if let Some(labels) = &labels
        && !labels::valid(db, user, identity, labels).await?
    {
        return Err(invalid());
    }
    let refs: Vec<Dependency> = serde_json::from_str(&manifest).map_err(|_| invalid())?;
    if refs.len() != model_count + 1 {
        return Err(invalid());
    }
    let mut models = Vec::new();
    let mut group = None;
    for reference in refs {
        let (id,guard,materialization,proof,wire):(i64,String,String,String,String)=sqlx::query_as("SELECT b.local_id,b.local_guard,b.materialization,b.observation_evidence,e.envelope FROM causal_observation_batch_members m JOIN causal_genesis_bindings b USING(user_id,entity_kind,birth_id) JOIN causal_events e USING(user_id,writer_id,counter) WHERE m.user_id=? AND m.batch_id=? AND m.entity_kind=? AND m.birth_id=?")
            .bind(user.id).bind(batch_id).bind(kind(&reference.entity)).bind(&reference.entity.birth_id).fetch_one(&mut *db).await?;
        let root = crate::causal_envelope::parse_causal_envelope(&wire).map_err(|_| invalid())?;
        if root.entity != reference.entity || root.event != reference.event {
            return Err(invalid());
        }
        super::validate(&root, &materialization, &proof, identity)?;
        let current = observed_guard(db, user, &reference.entity, id).await?;
        if serde_json::from_str::<serde_json::Value>(&guard).map_err(|_| invalid())?
            != serde_json::to_value(current).map_err(|_| invalid())?
        {
            return Err(invalid());
        }
        let unchanged:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM causal_genesis_bindings b WHERE b.user_id=? AND b.entity_kind=? AND b.birth_id=? AND b.local_observation_revision=CASE WHEN b.entity_kind='group' THEN (SELECT revision FROM group_observation_versions WHERE group_id=b.local_id) ELSE (SELECT revision FROM model_observation_versions WHERE model_id=b.local_id) END)")
            .bind(user.id).bind(kind(&reference.entity)).bind(&reference.entity.birth_id).fetch_one(&mut *db).await?;
        if !unchanged {
            return Err(invalid());
        }
        checks::uncontested(db, user, &reference.entity).await?;
        let value = serde_json::from_str(&materialization).map_err(|_| invalid())?;
        let bundle = GenesisBundle {
            envelope: root,
            wire,
            source_identity: identity.clone(),
            materialization: value,
            materialization_json: materialization,
            capability: "group_lifecycle_v1".into(),
            observation_evidence: Some(proof),
        };
        if reference.entity.kind == EntityKind::Group {
            group = Some(bundle);
        } else {
            models.push(bundle);
        }
    }
    Ok(Some(assemble(
        batch_id.to_owned(),
        group.ok_or_else(invalid)?,
        models,
        labels,
        project,
    )?))
}

pub(super) async fn install(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    peer: &crate::sync_peer::PeerBinding,
    bundle: &ObservedPopulatedGroupBundle,
    blobs: &[ModelBlobBinding],
) -> Result<PopulatedGroupInstallation, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut tx, user, identity).await?;
    checks::peer(&mut tx, user, identity, peer, &bundle.group).await?;
    if bundle.verified_origin.is_some() {
        return Err(invalid());
    }
    let result = install_transaction(&mut tx, user, identity, bundle, blobs).await?;
    tx.commit().await?;

    Ok(result)
}

#[allow(clippy::too_many_lines)]
pub(super) async fn install_transaction(
    tx: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    bundle: &ObservedPopulatedGroupBundle,
    blobs: &[ModelBlobBinding],
) -> Result<PopulatedGroupInstallation, DbError> {
    let blob_ids = validate_blobs(tx, bundle, blobs).await?;
    if let Some(retry) = install_retry(&mut *tx, user, identity, bundle, &blob_ids).await? {
        return Ok(retry);
    }
    for member in bundle.models.iter().chain(std::iter::once(&bundle.group)) {
        checks::empty_target(&mut *tx, user, member).await?;
    }
    write_batch(&mut *tx, user, identity, bundle, "installed").await?;
    for member in bundle.models.iter().chain(std::iter::once(&bundle.group)) {
        crate::causal_store::store_checked_receipt(
            &mut *tx,
            user.id,
            &member.envelope,
            &member.wire,
        )
        .await?;
    }
    let (group_id, _) = materialization::install(
        &mut *tx,
        user,
        bundle.group.entity(),
        &bundle.group.materialization,
        None,
    )
    .await?;
    install_members(tx, user, identity, bundle, group_id, blob_ids).await
}

pub(super) async fn validate_blobs(
    tx: &mut SqliteConnection,
    bundle: &ObservedPopulatedGroupBundle,
    blobs: &[ModelBlobBinding],
) -> Result<Vec<i64>, DbError> {
    if blobs.len() != bundle.models.len() {
        return Err(invalid());
    }
    let mut blob_ids = Vec::new();
    for model in &bundle.models {
        let matches: Vec<_> = blobs
            .iter()
            .filter(|mapping| mapping.entity == *model.entity())
            .collect();
        if matches.len() != 1 {
            return Err(invalid());
        }
        let id = matches[0].local_blob_id;
        materialization::validate_blob(&mut *tx, &model.materialization, Some(id)).await?;
        blob_ids.push(id);
    }

    Ok(blob_ids)
}

pub(super) async fn install_members(
    tx: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    bundle: &ObservedPopulatedGroupBundle,
    group_id: i64,
    blob_ids: Vec<i64>,
) -> Result<PopulatedGroupInstallation, DbError> {
    let models = install_member_rows(tx, user, identity, bundle, Some(group_id), blob_ids).await?;
    Ok(PopulatedGroupInstallation {
        group_id,
        models,
        already_present: false,
    })
}

pub(super) async fn install_member_rows(
    tx: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    bundle: &ObservedPopulatedGroupBundle,
    group_id: Option<i64>,
    blob_ids: Vec<i64>,
) -> Result<Vec<(Entity, i64)>, DbError> {
    if group_id.is_none() && bundle.identity_link.is_none() {
        return Err(invalid());
    }
    if let Some(group_id) = group_id {
        project::install_association(
            &mut *tx,
            user,
            bundle.receipt_source(),
            bundle.project.as_ref(),
            group_id,
        )
        .await?;
    }
    let mut installed = Vec::new();
    for (model, blob_id) in bundle.models.iter().zip(blob_ids) {
        let (id, _) = materialization::install(
            &mut *tx,
            user,
            model.entity(),
            &model.materialization,
            Some(blob_id),
        )
        .await?;
        sqlx::query("UPDATE models SET model_group_id=? WHERE model_id=? AND model_user_id=?")
            .bind(group_id)
            .bind(id)
            .bind(user.id)
            .execute(&mut *tx)
            .await?;
        installed.push((model.entity().clone(), id));
    }
    if let Some(labels) = &bundle.labels {
        labels::install(&mut *tx, user, bundle.receipt_source(), labels, &installed).await?;
    }
    for (model, (_, id)) in bundle.models.iter().zip(&installed) {
        let guard = observed_guard(&mut *tx, user, model.entity(), *id).await?;
        storage::write_binding(&mut *tx, user, identity, model, *id, &guard, "installed").await?;
    }
    if bundle.identity_link.is_none() {
        let group_id = group_id.ok_or_else(invalid)?;
        let guard =
            materialization::current_guard(&mut *tx, user, bundle.group.entity(), group_id).await?;
        storage::write_binding(
            &mut *tx,
            user,
            identity,
            &bundle.group,
            group_id,
            &guard,
            "installed",
        )
        .await?;
    }
    write_members(&mut *tx, user, bundle).await?;
    Ok(installed)
}

async fn install_retry(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    bundle: &ObservedPopulatedGroupBundle,
    blobs: &[i64],
) -> Result<Option<PopulatedGroupInstallation>, DbError> {
    let stored:Option<(String,String,String,String,String,String)>=sqlx::query_as("SELECT manifest,local_instance_id,local_account_id,source_instance_id,source_account_id,origin FROM causal_observation_batches WHERE user_id=? AND batch_id=?")
        .bind(user.id).bind(&bundle.batch_id).fetch_optional(&mut *db).await?;
    let Some((manifest, instance, account, source_instance, source_account, origin)) = stored
    else {
        return Ok(None);
    };
    let namespace: (Option<String>,Option<String>) = sqlx::query_as("SELECT verified_source_instance,verified_source_account FROM causal_observation_batches WHERE user_id=? AND batch_id=?").bind(user.id).bind(&bundle.batch_id).fetch_optional(&mut *db).await?.unwrap_or((None,None));
    if namespace.0.as_deref()
        != bundle
            .verified_origin
            .as_ref()
            .map(|source| source.instance_id.as_str())
        || namespace.1.as_deref()
            != bundle
                .verified_origin
                .as_ref()
                .map(|source| source.account_id.as_str())
    {
        return Err(invalid());
    }
    if manifest != bundle.manifest
        || instance != identity.instance_id
        || account != identity.account_id
        || source_instance != bundle.source_identity().instance_id
        || source_account != bundle.source_identity().account_id
        || origin != "installed"
    {
        return Err(invalid());
    }
    if let Some(labels) = &bundle.labels
        && !labels::valid(db, user, bundle.receipt_source(), labels).await?
    {
        return Err(invalid());
    }
    if let Some(project) = &bundle.project
        && !project::valid(db, user, bundle.receipt_source(), project).await?
    {
        return Err(invalid());
    }
    let group = storage::binding(db, user, bundle.group.entity())
        .await?
        .ok_or_else(invalid)?;
    group.compare(identity, &bundle.group, "installed")?;
    let group_id = storage::validate_retry(db, user, &bundle.group, &group, None).await?;
    let mut models = Vec::new();
    for (model, blob_id) in bundle.models.iter().zip(blobs) {
        let binding = storage::binding(db, user, model.entity())
            .await?
            .ok_or_else(invalid)?;
        binding.compare(identity, model, "installed")?;
        let id = storage::validate_retry(db, user, model, &binding, Some(*blob_id)).await?;
        models.push((model.entity().clone(), id));
    }
    Ok(Some(PopulatedGroupInstallation {
        group_id,
        models,
        already_present: true,
    }))
}
