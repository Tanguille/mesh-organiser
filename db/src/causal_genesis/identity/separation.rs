//! Explicit separation certifies a new namespace generation without rewriting old evidence.

use crate::causal_genesis::populated;

use super::{Authorization, DbError, Dot, invalid};

#[derive(Debug)]
pub struct GroupSeparationOutcome {
    pub local_group_id: i64,
    pub remote_group_id: i64,
    pub event: Dot,
    pub already_present: bool,
    pub outbox_id: Option<String>,
}

#[derive(sqlx::FromRow, serde::Serialize)]
struct OriginalHead {
    group_global_id: String,
    generation: i64,
    head: i64,
    state: String,
    identity_birth_id: Option<String>,
    last_deletion_operation_id: Option<String>,
}

pub(in crate::causal_genesis) async fn unmapped_history(
    db: &mut sqlx::SqliteConnection,
    user_id: i64,
    global_id: &str,
) -> Result<(), DbError> {
    // An empty group can still be named by a model's retained membership history.
    // Rekeying cannot assign that evidence to the incoming group's lineage. Inspect
    // every typed entity, including action targets and unresolved dependencies.
    // Prototype batch journals have no decoder that can establish their lineage;
    // any owner journal (even unrelated or malformed) excludes this narrow path.
    let history: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM group_causal_events WHERE user_id=?1 AND group_global_id=?2)
        OR EXISTS(SELECT 1 FROM assignment_batch_journal WHERE user_id=?1)
        OR EXISTS(SELECT 1 FROM received_group_renames WHERE user_id=?1 AND group_global_id=?2)
        OR EXISTS(SELECT 1 FROM membership_causal_event_facts WHERE user_id=?1
            AND (group_global_id=?2 OR from_group_global_id=?2 OR to_group_global_id=?2))
        OR EXISTS(SELECT 1 FROM causal_events e, json_tree(e.envelope) reference
            WHERE e.user_id=?1
            AND json_extract(CASE WHEN reference.type='object' THEN reference.value ELSE '{}' END,'$.kind')='group'
            AND json_extract(CASE WHEN reference.type='object' THEN reference.value ELSE '{}' END,'$.global_id')=?2)",
    )
    .bind(user_id)
    .bind(global_id)
    .fetch_one(&mut *db)
    .await?;
    if history {
        return Err(invalid());
    }

    Ok(())
}

pub async fn separate(
    db: &super::DbContext,
    auth: &Authorization,
    decision_id: &str,
) -> Result<GroupSeparationOutcome, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let result = separate_in(&mut tx, auth, decision_id, &[], None).await?;
    tx.commit().await?;

    Ok(result)
}

#[allow(clippy::too_many_lines)] // Rekey, retained evidence, both observations and receipt are one atomic choice.
pub(super) async fn separate_in(
    tx: &mut sqlx::SqliteConnection,
    auth: &Authorization,
    decision_id: &str,
    blobs: &[populated::ModelBlobBinding],
    local_source: Option<&super::LocalObservationStage>,
) -> Result<GroupSeparationOutcome, DbError> {
    let row = super::choice(&mut *tx, auth, decision_id).await?;
    if let (Some(writer_id), Some(counter)) = (&row.writer_id, row.counter) {
        let (remote_group_id, outbox_id): (i64, Option<String>) = sqlx::query_as("SELECT remote_group_id,local_outbox_id FROM group_identity_separations WHERE decision_id=? AND user_id=?")
            .bind(decision_id).bind(auth.user.id).fetch_optional(&mut *tx).await?.ok_or_else(invalid)?;

        if let Some(id) = &outbox_id {
            crate::sync_transport::outbox::require_retained(
                tx,
                auth,
                id,
                &Dot {
                    writer_id: writer_id.clone(),
                    counter: counter.to_string(),
                },
            )
            .await?;
        }

        return Ok(GroupSeparationOutcome {
            local_group_id: row.local_group_id,
            remote_group_id,
            event: Dot {
                writer_id: writer_id.clone(),
                counter: counter.to_string(),
            },
            already_present: true,
            outbox_id,
        });
    }
    let (delivery, plan, snapshot) = super::pending(&mut *tx, auth, &row).await?;
    separate_validated(
        tx,
        auth,
        decision_id,
        row.local_group_id,
        delivery,
        plan,
        snapshot,
        blobs,
        local_source,
    )
    .await
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Caller has checked the original group and every child in this same write transaction.
pub(super) async fn separate_validated(
    tx: &mut sqlx::SqliteConnection,
    auth: &Authorization,
    decision_id: &str,
    local_group_id: i64,
    delivery: super::Delivery,
    mut plan: crate::sync_transport::observations::ObservationPlan,
    snapshot: populated::IdentitySnapshot,
    blobs: &[populated::ModelBlobBinding],
    local_source: Option<&super::LocalObservationStage>,
) -> Result<GroupSeparationOutcome, DbError> {
    unmapped_history(tx, auth.user.id, &snapshot.group.global_id).await?;
    let populated_local = snapshot.has_models();
    if populated_local != local_source.is_some() {
        return Err(invalid());
    }
    let snapshot = snapshot.group;
    let old: OriginalHead = sqlx::query_as("SELECT group_global_id,generation,head,state,identity_birth_id,last_deletion_operation_id FROM group_sync_heads WHERE user_id=? AND group_global_id=?")
        .bind(auth.user.id).bind(&snapshot.global_id).fetch_one(&mut *tx).await?;
    if old.generation != snapshot.generation
        || old.head != snapshot.head
        || old.state != "active"
        || old.identity_birth_id.is_some()
        || old.last_deletion_operation_id.is_some()
    {
        return Err(invalid());
    }
    let remote_generation = old.generation.checked_add(1).ok_or_else(invalid)?;
    // The retained old namespace records both departure and the new materialization.
    let remote_head = old.head.checked_add(2).ok_or_else(invalid)?;
    let original_head = super::encode(&old)?;
    let local_global_id = crate::random_hex_32();
    let occupied: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_sync_heads WHERE user_id=? AND group_global_id=?) OR EXISTS(SELECT 1 FROM models_group WHERE group_user_id=? AND group_unique_global_id=?) OR EXISTS(SELECT 1 FROM entity_births WHERE user_id=? AND entity_kind='group' AND global_id=?) OR EXISTS(SELECT 1 FROM causal_events WHERE user_id=? AND json_extract(envelope,'$.entity.kind')='group' AND json_extract(envelope,'$.entity.global_id')=?)")
        .bind(auth.user.id).bind(&local_global_id).bind(auth.user.id).bind(&local_global_id).bind(auth.user.id).bind(&local_global_id).bind(auth.user.id).bind(&local_global_id).fetch_one(&mut *tx).await?;
    if occupied {
        return Err(invalid());
    }
    let changed = sqlx::query("UPDATE models_group SET group_unique_global_id=? WHERE group_id=? AND group_user_id=? AND group_unique_global_id=? AND group_birth_id IS NULL")
        .bind(&local_global_id).bind(local_group_id).bind(auth.user.id).bind(&snapshot.global_id).execute(&mut *tx).await?.rows_affected();
    if changed != 1 {
        return Err(invalid());
    }
    let captured =
        populated::observe_in(&mut *tx, &auth.user, &auth.identity, local_group_id).await?;
    let local = populated::transport::enroll_source_in(
        &mut *tx,
        &auth.user,
        &auth.identity,
        &captured,
        local_source.map_or(&[], |source| source.contents.as_slice()),
    )
    .await?;
    let outbox_id = if let Some(source) = local_source {
        Some(
            crate::sync_transport::outbox::queue_enrolled_observation_in(
                tx,
                auth,
                &local,
                &source.contents,
                &source.stage,
            )
            .await?
            .0,
        )
    } else {
        let item = crate::sync_transport::observations::encode(&local, vec![])?;
        Some(crate::sync_transport::outbox::queue_in(tx, auth, local.group_event(), &item).await?)
    };
    let remote = plan.bundle.identity_group()?;
    let remote_entity = remote.entity().clone();
    // A new row gets the normal next generation; the old head is never removed or reset.
    let (remote_group_id, _) = crate::causal_genesis::materialization::install(
        &mut *tx,
        &auth.user,
        &remote_entity,
        &remote.materialization,
        None,
    )
    .await?;
    let changed = sqlx::query("UPDATE group_sync_heads SET identity_birth_id=? WHERE user_id=? AND group_global_id=? AND generation=? AND head=? AND state='active' AND identity_birth_id IS NULL AND last_deletion_operation_id IS NULL")
        .bind(&remote_entity.birth_id).bind(auth.user.id).bind(&snapshot.global_id).bind(remote_generation).bind(remote_head).execute(&mut *tx).await?.rows_affected();
    if changed != 1 {
        return Err(invalid());
    }
    crate::causal_genesis::checks::uncontested(&mut *tx, &auth.user, &remote_entity).await?;
    populated::bind_existing_identity(
        &mut *tx,
        &auth.user,
        &auth.identity,
        auth.issuer_origin(&delivery.issuer),
        &mut plan.bundle,
        remote_group_id,
        blobs,
    )
    .await?;
    sqlx::query("INSERT INTO group_identity_separations(decision_id,user_id,original_head,local_global_id,local_birth_id,remote_group_id,remote_global_id,remote_birth_id,remote_generation,remote_head,local_outbox_id) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
        .bind(decision_id).bind(auth.user.id).bind(original_head).bind(&local_global_id).bind(&local.group_entity().birth_id).bind(remote_group_id).bind(&remote_entity.global_id).bind(&remote_entity.birth_id).bind(remote_generation).bind(remote_head).bind(&outbox_id).execute(&mut *tx).await?;
    crate::causal_group_lifecycle::project_identity_observation(&mut *tx, &auth.user).await?;
    delivery
        .commit(
            &mut *tx,
            "observed_group",
            populated::transport::batch_id(&plan.bundle),
        )
        .await?;
    let event = local.group_event().clone();
    sqlx::query("UPDATE sync_group_identity_choices SET writer_id=?,counter=?,local_outbox_id=? WHERE decision_id=? AND user_id=? AND writer_id IS NULL")
        .bind(&event.writer_id).bind(super::counter(&event)?).bind(&outbox_id).bind(decision_id).bind(auth.user.id).execute(&mut *tx).await?;

    Ok(GroupSeparationOutcome {
        local_group_id,
        remote_group_id,
        event,
        already_present: false,
        outbox_id,
    })
}
