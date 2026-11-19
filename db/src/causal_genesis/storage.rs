use sqlx::SqliteConnection;

use crate::{DbError, causal_store, model::user::User, user_db::SyncIdentity};

use super::{
    GenesisBundle, counter, encode, invalid, kind,
    materialization::{self, Guard},
};

#[derive(sqlx::FromRow)]
pub(super) struct Binding {
    global_id: String,
    writer_id: String,
    counter: i64,
    local_instance_id: String,
    local_account_id: String,
    source_instance_id: String,
    source_account_id: String,
    pub local_id: i64,
    pub local_guard: String,
    materialization: String,
    origin: String,
    capability: String,
    initialization_kind: String,
    observation_evidence: Option<String>,
}

impl Binding {
    pub(super) fn compare(
        &self,
        identity: &SyncIdentity,
        bundle: &GenesisBundle,
        origin: &str,
    ) -> Result<(), DbError> {
        if self.global_id != bundle.entity().global_id
            || self.writer_id != bundle.event().writer_id
            || self.counter != counter(bundle.event())?
            || self.local_instance_id != identity.instance_id
            || self.local_account_id != identity.account_id
            || self.source_instance_id != bundle.source_identity.instance_id
            || self.source_account_id != bundle.source_identity.account_id
            || self.materialization != bundle.materialization_json
            || self.origin != origin
            || self.capability != bundle.capability
            || self.initialization_kind
                != if bundle.observation_evidence.is_some() {
                    "observation"
                } else {
                    "creation"
                }
            || self.observation_evidence != bundle.observation_evidence
        {
            return Err(invalid());
        }

        Ok(())
    }
}

pub(super) async fn binding(
    db: &mut SqliteConnection,
    user: &User,
    entity: &crate::causal_envelope::Entity,
) -> Result<Option<Binding>, DbError> {
    sqlx::query_as("SELECT global_id, writer_id, counter, local_instance_id, local_account_id, source_instance_id, source_account_id, local_id, local_guard, materialization, origin, capability, initialization_kind, observation_evidence FROM causal_genesis_bindings WHERE user_id = ? AND entity_kind = ? AND birth_id = ? AND scope = 'rename_move_genesis_v1'")
        .bind(user.id).bind(kind(entity)).bind(&entity.birth_id).fetch_optional(db).await
}

pub(super) async fn validate_retry(
    db: &mut SqliteConnection,
    user: &User,
    bundle: &GenesisBundle,
    prior: &Binding,
    local_blob_id: Option<i64>,
) -> Result<i64, DbError> {
    if bundle.capability == "group_lifecycle_v1"
        && bundle.entity().kind == crate::causal_envelope::EntityKind::Group
    {
        // A verified keep may replace only the local materialization; an exact
        // genesis retry acknowledges that row without reinstalling or rewinding it.
        return sqlx::query_scalar("SELECT r.local_group_id FROM group_lifecycle_group_rows r JOIN group_lifecycle_scopes s ON s.user_id=r.user_id AND s.entity_kind='group' AND s.birth_id=r.birth_id JOIN models_group g ON g.group_id=r.local_group_id AND g.group_user_id=r.user_id AND g.group_birth_id=r.birth_id JOIN group_sync_heads h ON h.user_id=g.group_user_id AND h.group_global_id=g.group_unique_global_id WHERE r.user_id=? AND r.birth_id=? AND r.state='present' AND g.group_unique_global_id=? AND h.generation=json_extract(s.guard,'$.generation')")
            .bind(user.id).bind(&bundle.entity().birth_id).bind(&bundle.entity().global_id).fetch_optional(db).await?.ok_or_else(invalid);
    }
    let original: Guard = serde_json::from_str(&prior.local_guard).map_err(|_| invalid())?;
    let current = materialization::current_guard(db, user, bundle.entity(), prior.local_id).await?;
    // A retry acknowledges the original installation without rewinding later edits.
    // A purged/reused local incarnation must never be recreated by a stale bundle.
    let same = match (original, current) {
        (
            Guard::Group {
                generation: old, ..
            },
            Guard::Group {
                generation: new, ..
            },
        ) => old == new,
        (
            Guard::Model {
                incarnation: old,
                blob_id: old_blob,
                ..
            },
            Guard::Model {
                incarnation: new,
                blob_id: new_blob,
                ..
            },
        ) => old == new && old_blob == new_blob && Some(new_blob) == local_blob_id,
        _ => false,
    };
    if !same {
        return Err(invalid());
    }

    Ok(prior.local_id)
}

pub(super) async fn store_received_root(
    db: &mut SqliteConnection,
    user: &User,
    bundle: &GenesisBundle,
) -> Result<(), DbError> {
    let event = bundle.event();
    let local_writer: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM local_causal_writers WHERE replica_id = ?) OR EXISTS (SELECT 1 FROM group_causal_replica WHERE replica_id = ?)")
        .bind(&event.writer_id).bind(&event.writer_id).fetch_one(&mut *db).await?;
    if local_writer {
        return Err(invalid());
    }
    let prior: Option<(i64, String, String, bool)> = sqlx::query_as("SELECT e.user_id, e.encoding, e.envelope, EXISTS (SELECT 1 FROM causal_event_receipts r WHERE r.user_id = e.user_id AND r.writer_id = e.writer_id AND r.counter = e.counter) FROM causal_events e WHERE e.writer_id = ? AND e.counter = ?")
        .bind(&event.writer_id).bind(counter(event)?).fetch_optional(&mut *db).await?;
    if let Some((owner, encoding, wire, received)) = prior {
        if owner != user.id || encoding != "portable_v2" || wire != bundle.wire || !received {
            return Err(invalid());
        }

        return Ok(());
    }
    causal_store::check_quota(db, user.id, bundle.wire.len(), 0).await?;
    sqlx::query("INSERT OR IGNORE INTO causal_replica_owners (user_id, replica_id) VALUES (?, ?)")
        .bind(user.id)
        .bind(&event.writer_id)
        .execute(&mut *db)
        .await?;
    sqlx::query(
        "INSERT INTO causal_events (user_id, writer_id, counter, envelope) VALUES (?, ?, ?, ?)",
    )
    .bind(user.id)
    .bind(&event.writer_id)
    .bind(counter(event)?)
    .bind(&bundle.wire)
    .execute(&mut *db)
    .await?;
    sqlx::query("INSERT INTO causal_event_receipts (user_id, writer_id, counter) VALUES (?, ?, ?)")
        .bind(user.id)
        .bind(&event.writer_id)
        .bind(counter(event)?)
        .execute(db)
        .await?;

    Ok(())
}

pub(super) async fn write_binding(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    bundle: &GenesisBundle,
    local_id: i64,
    guard: &Guard,
    origin: &str,
) -> Result<(), DbError> {
    if bundle.capability == "group_lifecycle_v1" {
        check_combined_capacity(db, user, bundle, guard).await?;
    }
    let observation_revision: Option<i64> = if bundle.observation_evidence.is_some() {
        Some(
            sqlx::query_scalar("SELECT CASE WHEN ?='group' THEN (SELECT revision FROM group_observation_versions WHERE group_id=?) ELSE (SELECT revision FROM model_observation_versions WHERE model_id=?) END")
                .bind(kind(bundle.entity())).bind(local_id).bind(local_id)
                .fetch_one(&mut *db)
                .await?,
        )
    } else {
        None
    };
    sqlx::query("INSERT INTO causal_genesis_bindings (user_id, entity_kind, global_id, birth_id, writer_id, counter, local_instance_id, local_account_id, source_instance_id, source_account_id, local_id, local_guard, materialization, origin, capability, initialization_kind, observation_evidence, local_observation_revision) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(user.id).bind(kind(bundle.entity())).bind(&bundle.entity().global_id).bind(&bundle.entity().birth_id)
        .bind(&bundle.event().writer_id).bind(counter(bundle.event())?)
        .bind(&identity.instance_id).bind(&identity.account_id)
        .bind(&bundle.source_identity.instance_id).bind(&bundle.source_identity.account_id)
        .bind(local_id).bind(encode(guard)?).bind(&bundle.materialization_json).bind(origin).bind(&bundle.capability).bind(if bundle.observation_evidence.is_some(){"observation"}else{"creation"}).bind(&bundle.observation_evidence).bind(observation_revision).execute(db).await?;

    Ok(())
}

async fn check_combined_capacity(
    db: &mut SqliteConnection,
    user: &User,
    bundle: &GenesisBundle,
    guard: &Guard,
) -> Result<(), DbError> {
    let (count,bytes):(i64,i64)=sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(length(CAST(e.envelope AS BLOB))+length(CAST(b.materialization AS BLOB))+length(CAST(b.local_guard AS BLOB))+COALESCE(length(CAST(b.observation_evidence AS BLOB)),0)),0)+COALESCE((SELECT SUM(length(CAST(manifest AS BLOB))+COALESCE(length(CAST(label_snapshot AS BLOB)),0)+COALESCE(length(CAST(project_snapshot AS BLOB)),0)) FROM causal_observation_batches WHERE user_id=?),0) FROM causal_genesis_bindings b JOIN causal_events e ON e.user_id=b.user_id AND e.writer_id=b.writer_id AND e.counter=b.counter WHERE b.user_id=? AND b.capability='group_lifecycle_v1'")
        .bind(user.id).bind(user.id).fetch_one(db).await?;
    let bytes = usize::try_from(bytes)
        .map_err(|_| invalid())?
        .saturating_add(bundle.wire.len())
        .saturating_add(bundle.materialization_json.len())
        .saturating_add(bundle.observation_evidence.as_ref().map_or(0, String::len))
        .saturating_add(encode(guard)?.len());
    if usize::try_from(count).map_err(|_| invalid())? >= causal_store::MAX_OWNER_EVENTS
        || bytes > causal_store::MAX_OWNER_BYTES
    {
        return Err(invalid());
    }

    Ok(())
}
