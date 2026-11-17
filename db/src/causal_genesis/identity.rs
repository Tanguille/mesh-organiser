//! Explicit group identity choices preserve independent, incomplete observations.
pub mod model_pair;
mod names;
mod received_names;
pub use names::resolve_name_publication;
pub use received_names::{prepare_received_name, resolve_received_name};
pub(super) mod separation;
mod transfer;
use super::{counter, encode, invalid, observation, populated};
use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, Dot, Entity},
    db_context::DbContext,
    model::user::User,
    sync_transport::{
        observations,
        pull::{Authorization, Delivery, Issuer},
    },
    user_db::SyncIdentity,
};
pub use separation::{GroupSeparationOutcome, separate};
use sqlx::SqliteConnection;
pub use transfer::{
    Decision, DecisionOutcome, IdentityTransfer, LocalObservationStage, prepare_transfer,
};
pub(crate) use transfer::{finish_in, retry_in};

#[derive(Debug)]
pub enum NamePublication {
    Queued {
        outbox_id: String,
        event: Dot,
        already_present: bool,
        state: crate::causal_group_rename::NameState,
    },
    Stale {
        state: crate::causal_group_rename::NameState,
    },
}

#[derive(Debug)]
pub struct ReceivedNameChoice {
    pub choice_id: String,
    pub choices: Vec<crate::causal_group_rename::NameChoice>,
}

#[derive(sqlx::FromRow)]
struct ChoiceRecord {
    local_group_id: i64,
    delivery: String,
    item: String,
    snapshot: String,
    writer_id: Option<String>,
    counter: Option<i64>,
    local_outbox_id: Option<String>,
}

#[derive(Debug)]
pub struct GroupIdentityChoice {
    pub decision_id: String,
    pub local_name: String,
    pub remote_name: String,
    pub same_group_supported: bool,
    pub model_identity_required: bool,
    pub local_model_id: Option<i64>,
}
#[derive(Debug)]
pub struct GroupIdentityOutcome {
    pub group_id: i64,
    pub event: Dot,
    pub already_present: bool,
    pub outbox_id: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Position {
    stream: String,
    after: i64,
    through: i64,
    sequence: i64,
    digest: String,
    issuer: Issuer,
}
impl Position {
    fn capture(delivery: &Delivery) -> Self {
        Self {
            stream: delivery.stream.clone(),
            after: delivery.after,
            through: delivery.through,
            sequence: delivery.sequence,
            digest: delivery.digest.clone(),
            issuer: delivery.issuer.clone(),
        }
    }
    fn delivery(self, authorization: Authorization) -> Delivery {
        Delivery {
            authorization,
            stream: self.stream,
            after: self.after,
            through: self.through,
            sequence: self.sequence,
            digest: self.digest,
            issuer: self.issuer,
        }
    }
}

/// Discover only a question, never infer that two legacy groups are identical.
pub async fn prepare_next(
    db: &DbContext,
    delivery: &Delivery,
    value: &serde_json::Value,
) -> Result<Option<GroupIdentityChoice>, DbError> {
    if value.get("kind").and_then(serde_json::Value::as_str) != Some("observed_group") {
        return Ok(None);
    }
    let plan = observations::parse(value)?;
    if plan.digest != delivery.digest {
        return Err(invalid());
    }
    observations::author(&plan, &delivery.issuer.writer_id)?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    if delivery
        .validate(
            &mut tx,
            "observed_group",
            populated::transport::batch_id(&plan.bundle),
        )
        .await?
    {
        tx.commit().await?;

        return Ok(None);
    }
    let entity = plan.bundle.group_entity();
    let candidates: Vec<(i64, Option<String>)> = sqlx::query_as("SELECT group_id,group_birth_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=? LIMIT 2")
        .bind(delivery.authorization.user.id).bind(&entity.global_id).fetch_all(&mut *tx).await?;
    let candidate = match candidates.as_slice() {
        [] => None,
        [(group_id, None)] => Some(*group_id),
        [(_, Some(birth))] if Some(birth) == entity.birth_id.as_ref() => None,
        _ => return Err(invalid()),
    };
    tx.commit().await?;
    let Some(group_id) = candidate else {
        return Ok(None);
    };
    // Preparation rechecks the captured delivery and full candidate snapshot.
    // Unsupported children/history stop here; there is no fallback installation.
    prepare(db, delivery.clone(), value.clone(), group_id)
        .await
        .map(Some)
}

/// The HTTPS service supplies authenticated delivery evidence; a peer pin is insufficient.
pub async fn prepare(
    db: &DbContext,
    delivery: Delivery,
    value: serde_json::Value,
    group_id: i64,
) -> Result<GroupIdentityChoice, DbError> {
    prepare_impl(db, delivery, value, group_id, None).await
}

async fn prepare_impl(
    db: &DbContext,
    delivery: Delivery,
    value: serde_json::Value,
    group_id: i64,
    model_id: Option<i64>,
) -> Result<GroupIdentityChoice, DbError> {
    let plan = observations::parse(&value)?;
    let remote = plan.bundle.identity_group()?;
    if plan.digest != delivery.digest {
        return Err(invalid());
    }
    observations::author(&plan, &delivery.issuer.writer_id)?;
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    if delivery
        .validate(
            &mut tx,
            "observed_group",
            populated::transport::batch_id(&plan.bundle),
        )
        .await?
    {
        return Err(invalid());
    }
    let user = &delivery.authorization.user;
    let snapshot = populated::IdentitySnapshot::read(&mut tx, user, group_id).await?;
    if let Some(model_id) = model_id {
        model_pair::validate_pair(&mut tx, user, &snapshot, &plan, model_id).await?;
    } else {
        plan.bundle.novel_models(&mut tx, user).await?;
    }
    if snapshot.has_models() {
        separation::unmapped_history(&mut tx, user.id, &snapshot.group.global_id).await?;
    }
    if snapshot.group.birth_id.is_some() || snapshot.group.global_id != remote.entity().global_id {
        return Err(invalid());
    }
    let remote_name = match &remote.materialization {
        super::materialization::Materialization::Group(group) => group.name.clone(),
        super::materialization::Materialization::Model(_) => return Err(invalid()),
    };
    let decision_id = crate::random_hex_32();
    let authorization = delivery.authorization.identity_choice_key();
    let position = encode(&Position::capture(&delivery))?;
    let wire = encode(&value)?;
    let captured = encode(&snapshot)?;
    let same_group_supported = model_id.is_none();
    let prior: Option<String> = sqlx::query_scalar("SELECT decision_id FROM sync_group_identity_choices WHERE user_id=? AND local_group_id=? AND authorization=? AND delivery=? AND item=? AND snapshot=? AND writer_id IS NULL")
        .bind(user.id).bind(group_id).bind(&authorization).bind(&position).bind(&wire).bind(&captured).fetch_optional(&mut *tx).await?;
    if let Some(decision_id) = prior {
        let prior_model: Option<i64> = sqlx::query_scalar("SELECT local_model_id FROM sync_model_identity_choices WHERE decision_id=? AND user_id=?")
            .bind(&decision_id).bind(user.id).fetch_optional(&mut *tx).await?;
        if prior_model != model_id {
            return Err(invalid());
        }
        tx.commit().await?;
        return Ok(GroupIdentityChoice {
            decision_id,
            local_name: snapshot.group.name,
            remote_name,
            same_group_supported,
            model_identity_required: model_id.is_some(),
            local_model_id: model_id,
        });
    }
    // Superseded prompts have no domain effects, while committed receipts remain immutable.
    sqlx::query("DELETE FROM sync_group_identity_choices WHERE user_id=? AND local_group_id=? AND writer_id IS NULL")
        .bind(user.id).bind(group_id).execute(&mut *tx).await?;
    let (count, bytes): (i64, i64) = sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(length(CAST(authorization AS BLOB))+length(CAST(delivery AS BLOB))+length(CAST(item AS BLOB))+length(CAST(snapshot AS BLOB))),0) FROM sync_group_identity_choices WHERE user_id=?")
        .bind(user.id).fetch_one(&mut *tx).await?;
    if usize::try_from(count).map_err(|_| invalid())? >= crate::causal_store::MAX_OWNER_EVENTS
        || usize::try_from(bytes)
            .map_err(|_| invalid())?
            .saturating_add(authorization.len())
            .saturating_add(position.len())
            .saturating_add(wire.len())
            .saturating_add(captured.len())
            > crate::causal_store::MAX_OWNER_BYTES
    {
        return Err(invalid());
    }
    sqlx::query("INSERT INTO sync_group_identity_choices(decision_id,user_id,local_group_id,authorization,delivery,item,snapshot) VALUES(?,?,?,?,?,?,?)")
        .bind(&decision_id).bind(user.id).bind(group_id).bind(authorization).bind(position).bind(wire).bind(captured).execute(&mut *tx).await?;
    if let Some(model_id) = model_id {
        let entity = plan.bundle.models[0].entity();
        sqlx::query("INSERT INTO sync_model_identity_choices(decision_id,user_id,local_model_id,remote_global_id,remote_birth_id) VALUES(?,?,?,?,?)")
            .bind(&decision_id).bind(user.id).bind(model_id).bind(&entity.global_id).bind(&entity.birth_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;

    Ok(GroupIdentityChoice {
        decision_id,
        local_name: snapshot.group.name,
        remote_name,
        same_group_supported,
        model_identity_required: model_id.is_some(),
        local_model_id: model_id,
    })
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IndependentObservation {
    pub observation_id: String,
    pub baseline: Dot,
    pub root: String,
    pub source_identity: SyncIdentity,
    pub materialization: String,
    pub proof: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
}
impl IndependentObservation {
    pub(crate) fn validate(&self) -> Result<(CausalEnvelope, String), DbError> {
        crate::sync_peer::validate_identity(&self.source_identity)?;
        causal_envelope::counter(&self.baseline).map_err(|_| invalid())?;
        if self.observation_id.len() != 32
            || !self.observation_id.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid());
        }
        let root = causal_envelope::parse_causal_envelope(&self.root).map_err(|_| invalid())?;
        if causal_envelope::canonical_causal_envelope(&root).map_err(|_| invalid())? != self.root {
            return Err(invalid());
        }
        if let Some(batch_id) = &self.batch_id {
            let proof: serde_json::Value =
                serde_json::from_str(&self.proof).map_err(|_| invalid())?;
            if proof["batch_id"] != *batch_id
                || proof["kind"] != "populated_group"
                || !proof["original"]["birth_id"].is_null()
            {
                return Err(invalid());
            }
            let (name, destination) = populated::validate(
                &root,
                &self.materialization,
                &self.proof,
                &self.source_identity,
            )?;
            if destination.is_some() {
                return Err(invalid());
            }
            return Ok((root, name.ok_or_else(invalid)?));
        }
        let name = observation::validate(
            &root,
            &self.materialization,
            &self.proof,
            &self.source_identity,
        )?;
        let proof: serde_json::Value = serde_json::from_str(&self.proof).map_err(|_| invalid())?;
        if proof["observation_id"] != self.observation_id
            || !proof["snapshot"]["birth_id"].is_null()
        {
            return Err(invalid());
        }
        Ok((root, name))
    }
    pub(crate) fn item(&self) -> serde_json::Value {
        serde_json::json!({"kind":"group_identity_observation", "observation": self})
    }
}

/// Cancel only a pending prompt. A fresh prompt gets a fresh token even if the data is unchanged.
pub async fn cancel(
    db: &DbContext,
    auth: &Authorization,
    decision_id: &str,
) -> Result<(), DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let row = choice(&mut tx, auth, decision_id).await?;
    if row.writer_id.is_some() || row.counter.is_some() {
        return Err(invalid());
    }
    let removed=sqlx::query("DELETE FROM sync_group_identity_choices WHERE decision_id=? AND user_id=? AND writer_id IS NULL AND counter IS NULL")
        .bind(decision_id).bind(auth.user.id).execute(&mut *tx).await?.rows_affected();
    if removed != 1 {
        return Err(invalid());
    }
    tx.commit().await?;
    Ok(())
}

async fn choice(
    db: &mut SqliteConnection,
    auth: &Authorization,
    decision_id: &str,
) -> Result<ChoiceRecord, DbError> {
    auth.validate(db).await?;
    sqlx::query_as("SELECT local_group_id,delivery,item,snapshot,writer_id,counter,local_outbox_id FROM sync_group_identity_choices WHERE decision_id=? AND user_id=? AND authorization=?")
        .bind(decision_id).bind(auth.user.id).bind(auth.identity_choice_key()).fetch_optional(db).await?.ok_or_else(invalid)
}

async fn pending(
    db: &mut SqliteConnection,
    auth: &Authorization,
    row: &ChoiceRecord,
) -> Result<
    (
        Delivery,
        observations::ObservationPlan,
        populated::IdentitySnapshot,
    ),
    DbError,
> {
    let position: Position = serde_json::from_str(&row.delivery).map_err(|_| invalid())?;
    let delivery = position.delivery(auth.clone());
    let value = serde_json::from_str(&row.item).map_err(|_| invalid())?;
    let plan = observations::parse(&value)?;
    let remote = plan.bundle.identity_group()?;
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
    plan.bundle.novel_models(db, &auth.user).await?;
    let snapshot: populated::IdentitySnapshot =
        serde_json::from_str(&row.snapshot).map_err(|_| invalid())?;
    if snapshot != populated::IdentitySnapshot::read(db, &auth.user, row.local_group_id).await?
        || snapshot.group.birth_id.is_some()
        || snapshot.group.global_id != remote.entity().global_id
    {
        return Err(invalid());
    }
    Ok((delivery, plan, snapshot))
}

pub(crate) async fn verify_transfer_in(
    db: &mut SqliteConnection,
    auth: &Authorization,
    decision_id: &str,
    delivery: &Delivery,
) -> Result<(), DbError> {
    let row = choice(db, auth, decision_id).await?;
    if row.delivery != encode(&Position::capture(delivery))? {
        return Err(invalid());
    }

    Ok(())
}

pub async fn confirm(
    db: &DbContext,
    auth: &Authorization,
    decision_id: &str,
) -> Result<GroupIdentityOutcome, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let result = confirm_in(&mut tx, auth, decision_id, &[], None).await?;
    tx.commit().await?;

    Ok(result)
}

#[allow(clippy::too_many_lines)] // Consent, both observations and the incoming receipt commit atomically.
pub(crate) async fn confirm_in(
    tx: &mut SqliteConnection,
    auth: &Authorization,
    decision_id: &str,
    blobs: &[populated::ModelBlobBinding],
    local_source: Option<&LocalObservationStage>,
) -> Result<GroupIdentityOutcome, DbError> {
    let row = choice(&mut *tx, auth, decision_id).await?;
    let separated: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM group_identity_separations WHERE decision_id=? AND user_id=?)",
    )
    .bind(decision_id)
    .bind(auth.user.id)
    .fetch_one(&mut *tx)
    .await?;
    if separated {
        return Err(invalid());
    }
    let group_id = row.local_group_id;
    if let (Some(writer_id), Some(number)) = (&row.writer_id, row.counter) {
        if let Some(id) = &row.local_outbox_id {
            crate::sync_transport::outbox::require_retained(
                tx,
                auth,
                id,
                &Dot {
                    writer_id: writer_id.clone(),
                    counter: number.to_string(),
                },
            )
            .await?;
        }
        return Ok(GroupIdentityOutcome {
            group_id,
            event: Dot {
                writer_id: writer_id.clone(),
                counter: number.to_string(),
            },
            already_present: true,
            outbox_id: row.local_outbox_id,
        });
    }
    let (delivery, mut plan, snapshot) = pending(&mut *tx, auth, &row).await?;
    if snapshot.has_models() {
        let source = local_source.ok_or_else(invalid)?;
        return populated::enroll_identity(
            tx,
            auth,
            decision_id,
            group_id,
            delivery,
            plan,
            blobs,
            source,
        )
        .await;
    }
    if local_source.is_some() {
        return Err(invalid());
    }
    let snapshot = snapshot.group;
    let remote = plan.bundle.identity_group()?;
    let entity = remote.entity().clone();
    let baseline = remote.event().clone();
    let capture = observation::GroupObservation {
        owner: auth.user.id,
        identity: auth.identity.clone(),
        group_id,
        snapshot: snapshot.clone(),
        observation_id: decision_id.to_owned(),
    };
    let (proof, _, materialization) = observation::evidence(&capture)?;
    sqlx::query("INSERT INTO group_birth_adoption_permits(group_id,user_id,account_id,instance_id,global_id,birth_id,observation_revision,head) VALUES(?,?,?,?,?,?,?,?)")
        .bind(group_id).bind(auth.user.id).bind(&auth.identity.account_id).bind(&auth.identity.instance_id).bind(&snapshot.global_id).bind(&entity.birth_id).bind(snapshot.epoch).bind(snapshot.head).execute(&mut *tx).await?;
    sqlx::query("UPDATE models_group SET group_birth_id=? WHERE group_id=? AND group_user_id=? AND group_birth_id IS NULL")
        .bind(&entity.birth_id).bind(group_id).bind(auth.user.id).execute(&mut *tx).await?;
    populated::bind_existing_identity(
        &mut *tx,
        &auth.user,
        &auth.identity,
        auth.issuer_origin(&delivery.issuer),
        &mut plan.bundle,
        group_id,
        blobs,
    )
    .await?;
    let (writer_id, number) = crate::causal_clock::next(&mut *tx, auth.user.id).await?;
    let root = CausalEnvelope {
        version: 2,
        entity,
        event: Dot {
            writer_id,
            counter: number.to_string(),
        },
        parents: vec![],
        dependencies: vec![],
        action: Action::LegacyObserved {
            observation_digest: observation::digest(&proof),
        },
    };
    let item = IndependentObservation {
        observation_id: decision_id.to_owned(),
        baseline,
        root: causal_envelope::canonical_causal_envelope(&root).map_err(|_| invalid())?,
        source_identity: auth.identity.clone(),
        materialization,
        proof,
        batch_id: None,
    };
    insert(&mut *tx, &auth.user, &item, true).await?;
    crate::causal_group_lifecycle::project_identity_observation(&mut *tx, &auth.user).await?;
    let outbox_id =
        crate::sync_transport::outbox::queue_in(tx, auth, &root.event, &item.item()).await?;
    delivery
        .commit(
            &mut *tx,
            "observed_group",
            populated::transport::batch_id(&plan.bundle),
        )
        .await?;
    sqlx::query("UPDATE sync_group_identity_choices SET writer_id=?,counter=?,local_outbox_id=? WHERE decision_id=? AND user_id=? AND writer_id IS NULL")
        .bind(&root.event.writer_id).bind(number).bind(&outbox_id).bind(decision_id).bind(auth.user.id).execute(&mut *tx).await?;

    Ok(GroupIdentityOutcome {
        group_id,
        event: root.event,
        already_present: false,
        outbox_id: Some(outbox_id),
    })
}

pub(crate) async fn insert(
    db: &mut SqliteConnection,
    user: &User,
    value: &IndependentObservation,
    local: bool,
) -> Result<(), DbError> {
    let (root, _) = value.validate()?;
    let item = encode(&value.item())?;
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT item FROM group_identity_observations WHERE user_id=? AND observation_id=?",
    )
    .bind(user.id)
    .bind(&value.observation_id)
    .fetch_optional(&mut *db)
    .await?;
    if let Some(existing) = existing {
        return if existing == item {
            Ok(())
        } else {
            Err(invalid())
        };
    }
    // Plain roots contribute only a name. Populated roots enter through the full
    // batch installer, which validates existing lifecycle guards before mutation.
    // Their independent add may keep a concurrently deleted group alive.
    let supported: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM causal_genesis_bindings b JOIN group_lifecycle_scopes s USING(user_id,entity_kind,birth_id) JOIN group_lifecycle_group_rows r ON r.user_id=b.user_id AND r.birth_id=b.birth_id LEFT JOIN models_group g ON g.group_id=r.local_group_id AND g.group_user_id=b.user_id WHERE b.user_id=? AND b.entity_kind='group' AND b.birth_id=? AND b.global_id=? AND b.writer_id=? AND b.counter=? AND s.paused=0 AND ((r.state='present' AND g.group_birth_id=b.birth_id AND g.group_unique_global_id=b.global_id AND g.group_resource_id IS NULL AND ((g.group_deleted_at IS NULL AND g.group_deletion_operation_id IS NULL) OR ?)) OR (? AND r.state='purged' AND r.retained_resource_id IS NULL)))")
        .bind(user.id).bind(&root.entity.birth_id).bind(&root.entity.global_id).bind(&value.baseline.writer_id).bind(counter(&value.baseline)?).bind(value.batch_id.is_some()).bind(value.batch_id.is_some()).fetch_one(&mut *db).await?;
    if !supported {
        return Err(invalid());
    }
    let (count, bytes):(i64,i64) = sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(length(CAST(item AS BLOB))),0) FROM group_identity_observations WHERE user_id=?")
        .bind(user.id).fetch_one(&mut *db).await?;
    if usize::try_from(count).map_err(|_| invalid())? >= crate::causal_store::MAX_OWNER_EVENTS
        || usize::try_from(bytes)
            .map_err(|_| invalid())?
            .saturating_add(item.len())
            > crate::causal_store::MAX_OWNER_BYTES
    {
        return Err(invalid());
    }
    if local {
        sqlx::query(
            "INSERT INTO causal_events(user_id,writer_id,counter,envelope) VALUES(?,?,?,?)",
        )
        .bind(user.id)
        .bind(&root.event.writer_id)
        .bind(counter(&root.event)?)
        .bind(&value.root)
        .execute(&mut *db)
        .await?;
    } else {
        crate::causal_store::store_checked_receipt(db, user.id, &root, &value.root).await?;
    }
    sqlx::query("INSERT INTO group_identity_observations(user_id,observation_id,birth_id,writer_id,counter,item,batch_id) VALUES(?,?,?,?,?,?,?)")
        .bind(user.id).bind(&value.observation_id).bind(&root.entity.birth_id).bind(&root.event.writer_id).bind(counter(&root.event)?).bind(item).bind(&value.batch_id).execute(db).await?;
    Ok(())
}

#[derive(Clone)]
pub(crate) struct IndependentRoot {
    pub root: CausalEnvelope,
    pub name: Option<String>,
    pub destination: Option<Entity>,
    pub batch_id: Option<String>,
}

pub(crate) async fn roots(
    db: &mut SqliteConnection,
    user: &User,
    entity: &Entity,
) -> Result<Vec<IndependentRoot>, DbError> {
    if entity.kind != causal_envelope::EntityKind::Group {
        return populated::model_identity::roots(db, user, entity).await;
    }
    let values: Vec<String> = sqlx::query_scalar("SELECT item FROM group_identity_observations WHERE user_id=? AND birth_id=? ORDER BY observation_id LIMIT 4097")
        .bind(user.id).bind(&entity.birth_id).fetch_all(db).await?;
    if values.len() > crate::causal_store::MAX_OWNER_EVENTS {
        return Err(invalid());
    }
    values
        .into_iter()
        .map(|value| {
            let item: serde_json::Value = serde_json::from_str(&value).map_err(|_| invalid())?;
            let observation: IndependentObservation =
                serde_json::from_value(item["observation"].clone()).map_err(|_| invalid())?;
            let (root, name) = observation.validate()?;
            if root.entity != *entity {
                return Err(invalid());
            }
            Ok(IndependentRoot {
                root,
                name: Some(name),
                destination: None,
                batch_id: observation.batch_id,
            })
        })
        .collect()
}

pub(crate) async fn read(
    db: &mut SqliteConnection,
    user: &User,
    id: &str,
) -> Result<serde_json::Value, DbError> {
    let item: String = sqlx::query_scalar(
        "SELECT item FROM group_identity_observations WHERE user_id=? AND observation_id=?",
    )
    .bind(user.id)
    .bind(id)
    .fetch_one(db)
    .await?;
    serde_json::from_str(&item).map_err(|_| invalid())
}

pub async fn export(
    db: &DbContext,
    auth: &Authorization,
    event: &Dot,
) -> Result<serde_json::Value, DbError> {
    let mut tx = db.begin().await?;
    auth.validate(&mut tx).await?;
    let id: String = sqlx::query_scalar("SELECT observation_id FROM group_identity_observations WHERE user_id=? AND writer_id=? AND counter=?")
        .bind(auth.user.id).bind(&event.writer_id).bind(counter(event)?).fetch_one(&mut *tx).await?;
    let item = read(&mut tx, &auth.user, &id).await?;
    tx.commit().await?;
    Ok(item)
}
