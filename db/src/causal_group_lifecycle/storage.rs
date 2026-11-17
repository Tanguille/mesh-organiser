use sqlx::SqliteConnection;

use crate::{
    DbError,
    causal_envelope::{self, CausalEnvelope, Entity, EntityKind},
    causal_store,
    model::user::User,
};

use super::{counter, invalid};

#[derive(sqlx::FromRow)]
struct Row {
    entity_kind: String,
    birth_id: String,
    global_id: String,
    local_id: i64,
    guard: String,
    original_guard: String,
    materialization: String,
    revision: i64,
    paused: bool,
    wire: String,
    initialization_kind: String,
    observation_evidence: Option<String>,
    source_instance_id: String,
    source_account_id: String,
    materialization_state: Option<String>,
    observed_deleted: Option<String>,
    project_id: Option<i64>,
    project_incarnation: Option<String>,
}

type ObservedRoot = (Option<String>, Option<Entity>, Option<String>);

impl Row {
    fn observation(&self, root: &CausalEnvelope) -> Result<ObservedRoot, DbError> {
        let Some(proof) = self.observation_evidence.as_deref() else {
            if self.initialization_kind != "creation"
                || !matches!(
                    root.action,
                    causal_envelope::Action::GroupCreated { .. }
                        | causal_envelope::Action::ModelCreated
                )
            {
                return Err(invalid());
            }

            return Ok((None, None, None));
        };
        if self.initialization_kind != "observation" {
            return Err(invalid());
        }
        let source = crate::user_db::SyncIdentity {
            instance_id: self.source_instance_id.clone(),
            account_id: self.source_account_id.clone(),
        };
        let value: serde_json::Value = serde_json::from_str(proof).map_err(|_| invalid())?;
        if value["kind"].as_str().is_some() {
            let (name, destination) = crate::causal_genesis::populated::validate(
                root,
                &self.materialization,
                proof,
                &source,
            )?;

            return Ok((
                name,
                destination,
                Some(value["batch_id"].as_str().ok_or_else(invalid)?.to_string()),
            ));
        }
        Ok((
            Some(crate::causal_genesis::observation::validate(
                root,
                &self.materialization,
                proof,
                &source,
            )?),
            None,
            None,
        ))
    }
}

#[derive(Clone)]
pub(super) struct Scope {
    pub entity: Entity,
    pub local_id: i64,
    pub revision: i64,
    pub generation: i64,
    pub incarnation: String,
    pub paused: bool,
    pub root: CausalEnvelope,
    pub independent_observations: Vec<crate::causal_genesis::identity::IndependentRoot>,
    pub current_name: String,
    pub observed_name: Option<String>,
    pub observed_destination: Option<Entity>,
    pub observation_batch_id: Option<String>,
    pub purged: bool,
    pub current_destination: Option<Entity>,
    pub observed_deleted: Option<String>,
    pub project_id: Option<i64>,
    pub project_incarnation: Option<String>,
}

pub(super) async fn enrolled(
    db: &mut SqliteConnection,
    user: &User,
    kind: &str,
    id: i64,
) -> Result<bool, DbError> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM causal_genesis_bindings b LEFT JOIN group_lifecycle_group_rows r ON r.user_id=b.user_id AND r.birth_id=b.birth_id AND b.entity_kind='group' WHERE b.user_id=? AND b.entity_kind=? AND COALESCE(r.local_group_id,b.local_id)=? AND b.capability='group_lifecycle_v1')")
        .bind(user.id).bind(kind).bind(id).fetch_one(db).await
}

pub(super) async fn current_guard(
    db: &mut SqliteConnection,
    user: &User,
    scope: &Scope,
) -> Result<Option<String>, DbError> {
    if scope.purged {
        return super::retention::absence_guard(db, user, scope).await;
    }
    match scope.entity.kind {
        EntityKind::Group => sqlx::query_scalar("SELECT json_object('kind','group','generation',h.generation,'head',h.head,'revision',g.group_revision) FROM models_group g JOIN group_sync_heads h ON h.user_id=g.group_user_id AND h.group_global_id=g.group_unique_global_id WHERE g.group_id=? AND g.group_user_id=? AND g.group_unique_global_id=? AND g.group_birth_id=? AND g.group_resource_id IS ?")
            .bind(scope.local_id).bind(user.id).bind(&scope.entity.global_id).bind(&scope.entity.birth_id).bind(scope.project_id).fetch_optional(db).await,
        EntityKind::Model => sqlx::query_scalar("SELECT CASE WHEN ? THEN json_set(json_object('kind','model','incarnation',model_incarnation,'blob_id',model_blob_id,'assignment_revision',model_assignment_revision,'lifecycle_revision',model_lifecycle_revision),'$.observation_revision',(SELECT revision FROM model_observation_versions WHERE model_id=models.model_id)) ELSE json_object('kind','model','incarnation',model_incarnation,'blob_id',model_blob_id,'assignment_revision',model_assignment_revision,'lifecycle_revision',model_lifecycle_revision) END FROM models WHERE model_id=? AND model_user_id=? AND model_unique_global_id=? AND model_birth_id=? AND model_deleted IS ?")
            .bind(scope.observed_destination.is_some())
            .bind(scope.local_id).bind(user.id).bind(&scope.entity.global_id).bind(&scope.entity.birth_id).bind(&scope.observed_deleted).fetch_optional(db).await,
    }
}

fn guards_equal(current: Option<&str>, expected: &str) -> Result<bool, DbError> {
    match current {
        Some(current) => Ok(serde_json::from_str::<serde_json::Value>(current)
            .map_err(|_| invalid())?
            == serde_json::from_str::<serde_json::Value>(expected).map_err(|_| invalid())?),
        None => Ok(false),
    }
}

#[allow(clippy::too_many_lines)] // A scope is usable only after all materialization and observation guards agree.
pub(super) async fn load(db: &mut SqliteConnection, user: &User) -> Result<Vec<Scope>, DbError> {
    let (count, bytes):(i64,i64)=sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(length(CAST(e.envelope AS BLOB))+length(CAST(b.materialization AS BLOB))+length(CAST(b.local_guard AS BLOB))+COALESCE(length(CAST(b.observation_evidence AS BLOB)),0)),0)+COALESCE((SELECT SUM(length(CAST(manifest AS BLOB))+COALESCE(length(CAST(label_snapshot AS BLOB)),0)+COALESCE(length(CAST(project_snapshot AS BLOB)),0)) FROM causal_observation_batches WHERE user_id=?),0) FROM group_lifecycle_scopes s JOIN causal_genesis_bindings b USING(user_id,entity_kind,birth_id) JOIN causal_events e ON e.user_id=b.user_id AND e.writer_id=b.writer_id AND e.counter=b.counter WHERE s.user_id=?")
        .bind(user.id).bind(user.id).fetch_one(&mut *db).await?;
    if usize::try_from(count).map_err(|_| invalid())? > causal_store::MAX_OWNER_EVENTS
        || usize::try_from(bytes).map_err(|_| invalid())? > causal_store::MAX_OWNER_BYTES
    {
        return Err(invalid());
    }
    let rows: Vec<Row> = sqlx::query_as("SELECT s.entity_kind,s.birth_id,b.global_id,COALESCE(r.local_group_id,b.local_id) AS local_id,r.state AS materialization_state,s.guard,b.local_guard AS original_guard,b.materialization,json_extract(b.materialization,'$.deleted') AS observed_deleted,s.revision,s.paused,e.envelope AS wire,b.initialization_kind,b.observation_evidence,b.source_instance_id,b.source_account_id,p.local_resource_id AS project_id,p.local_incarnation AS project_incarnation FROM group_lifecycle_scopes s JOIN causal_genesis_bindings b USING(user_id,entity_kind,birth_id) LEFT JOIN causal_observation_batch_members bm ON bm.user_id=b.user_id AND bm.entity_kind=b.entity_kind AND bm.birth_id=b.birth_id LEFT JOIN causal_observation_batches ob ON ob.user_id=bm.user_id AND ob.batch_id=bm.batch_id LEFT JOIN observed_project_bindings p ON p.user_id=ob.user_id AND p.source_instance_id=COALESCE(ob.verified_source_instance,ob.source_instance_id) AND p.source_account_id=COALESCE(ob.verified_source_account,ob.source_account_id) AND p.source_incarnation=json_extract(ob.project_snapshot,'$.incarnation') LEFT JOIN group_lifecycle_group_rows r ON r.user_id=b.user_id AND r.birth_id=b.birth_id AND b.entity_kind='group' JOIN causal_events e ON e.user_id=b.user_id AND e.writer_id=b.writer_id AND e.counter=b.counter JOIN sync_accounts a ON a.user_id=s.user_id JOIN sync_instance i ON i.singleton=1 WHERE s.user_id=? AND b.capability='group_lifecycle_v1' AND b.local_instance_id=i.instance_id AND b.local_account_id=a.account_id AND e.encoding='portable_v2' LIMIT 4097")
        .bind(user.id).fetch_all(&mut *db).await?;
    if rows.len() > causal_store::MAX_OWNER_EVENTS {
        return Err(invalid());
    }
    let mut scopes = Vec::new();
    for row in rows {
        let root = causal_envelope::parse_causal_envelope(&row.wire).map_err(|_| invalid())?;
        let (observed_name, observed_destination, observation_batch_id) = row.observation(&root)?;
        let original: serde_json::Value =
            serde_json::from_str(&row.original_guard).map_err(|_| invalid())?;
        let mut scope = Scope {
            entity: Entity {
                kind: if row.entity_kind == "group" {
                    EntityKind::Group
                } else {
                    EntityKind::Model
                },
                global_id: row.global_id,
                birth_id: Some(row.birth_id),
            },
            local_id: row.local_id,
            revision: row.revision,
            generation: serde_json::from_str::<serde_json::Value>(&row.guard)
                .map_err(|_| invalid())?["generation"]
                .as_i64()
                .unwrap_or(0),
            incarnation: original["incarnation"].as_str().unwrap_or_default().into(),
            paused: row.paused,
            root,
            independent_observations: vec![],
            current_name: String::new(),
            observed_name,
            observed_destination,
            observation_batch_id,
            purged: row.materialization_state.as_deref() == Some("purged"),
            current_destination: None,
            observed_deleted: row.observed_deleted,
            project_id: row.project_id,
            project_incarnation: row.project_incarnation,
        };
        if scope.root.entity != scope.entity
            || !scope.root.parents.is_empty()
            || (scope.observation_batch_id.is_none() && !scope.root.dependencies.is_empty())
        {
            return Err(invalid());
        }
        scope.independent_observations =
            crate::causal_genesis::identity::roots(db, user, &scope.entity).await?;
        if scope.entity.kind == EntityKind::Group {
            scope.current_name = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=? AND group_user_id=? AND group_birth_id=?")
                .bind(scope.local_id).bind(user.id).bind(&scope.entity.birth_id).fetch_optional(&mut *db).await?.unwrap_or_default();
        } else {
            let target: Option<(String,Option<String>)> = sqlx::query_as("SELECT g.group_unique_global_id,g.group_birth_id FROM models m JOIN models_group g ON g.group_id=m.model_group_id AND g.group_user_id=m.model_user_id WHERE m.model_id=? AND m.model_user_id=? AND m.model_incarnation=?")
                .bind(scope.local_id).bind(user.id).bind(&scope.incarnation).fetch_optional(&mut *db).await?;
            scope.current_destination = target.map(|(global_id, birth_id)| Entity {
                kind: EntityKind::Group,
                global_id,
                birth_id,
            });
        }
        if scope.purged {
            scope.current_name=sqlx::query_scalar("SELECT retained_name FROM group_lifecycle_group_rows WHERE user_id=? AND birth_id=? AND state='purged'")
                .bind(user.id).bind(&scope.entity.birth_id).fetch_one(&mut *db).await?;
        }
        let current = current_guard(db, user, &scope).await?;
        let equal = guards_equal(current.as_deref(), &row.guard)?;
        let mut supported = equal
            && (scope.entity.kind != EntityKind::Group || row.materialization_state.is_some())
            && causal_store::identity_status(db, user.id, &scope.entity).await?
                == causal_store::IdentityStatus::Uncontested;
        if scope.entity.kind == EntityKind::Model {
            let materialization: serde_json::Value =
                serde_json::from_str(&row.materialization).map_err(|_| invalid())?;
            let descriptor = &materialization["blob"];
            let blob_matches: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM models m JOIN blobs b ON b.blob_id=m.model_blob_id WHERE m.model_id=? AND m.model_user_id=? AND m.model_incarnation=? AND m.model_blob_id=? AND b.blob_sha256=? AND b.blob_filetype=? AND b.blob_size=?)")
                .bind(scope.local_id).bind(user.id).bind(&scope.incarnation).bind(original["blob_id"].as_i64()).bind(descriptor["hash"].as_str()).bind(descriptor["filetype"].as_str()).bind(descriptor["size"].as_i64()).fetch_one(&mut *db).await?;
            supported &= blob_matches;
        }
        if !supported && !scope.paused {
            sqlx::query("UPDATE group_lifecycle_scopes SET paused=1 WHERE user_id=? AND entity_kind=? AND birth_id=?")
                .bind(user.id).bind(&row.entity_kind).bind(&scope.entity.birth_id).execute(&mut *db).await?;
            scope.paused = true;
        }
        scopes.push(scope);
    }

    validate_observed_metadata(db, user, &mut scopes).await?;

    if scopes.iter().any(|scope| scope.paused) {
        pause(db, user).await?;
        for scope in &mut scopes {
            scope.paused = true;
        }
    }

    Ok(scopes)
}

async fn validate_observed_metadata(
    db: &mut SqliteConnection,
    user: &User,
    scopes: &mut [Scope],
) -> Result<(), DbError> {
    let batches: std::collections::BTreeSet<_> = scopes
        .iter()
        .filter_map(|scope| scope.observation_batch_id.clone())
        .collect();
    for batch in batches {
        if !crate::causal_genesis::populated::labels_valid(db, user, &batch).await?
            || !crate::causal_genesis::populated::project_valid(db, user, &batch).await?
        {
            for scope in scopes
                .iter_mut()
                .filter(|scope| scope.observation_batch_id.as_ref() == Some(&batch))
            {
                scope.paused = true;
            }
        }
    }

    Ok(())
}

pub(super) async fn pause(db: &mut SqliteConnection, user: &User) -> Result<(), DbError> {
    sqlx::query("UPDATE group_lifecycle_scopes SET paused=1 WHERE user_id=?")
        .bind(user.id)
        .execute(db)
        .await?;

    Ok(())
}

pub(super) async fn events(
    db: &mut SqliteConnection,
    user: &User,
) -> Result<Vec<CausalEnvelope>, DbError> {
    let (count, bytes, edges):(i64,i64,i64) = sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(length(CAST(e.envelope AS BLOB))),0),COALESCE(SUM(json_array_length(e.envelope,'$.parents')+json_array_length(e.envelope,'$.dependencies')),0) FROM group_lifecycle_events m JOIN causal_events e USING(user_id,writer_id,counter) WHERE m.user_id=?")
        .bind(user.id).fetch_one(&mut *db).await?;
    let count = usize::try_from(count).map_err(|_| invalid())?;
    let bytes = usize::try_from(bytes).map_err(|_| invalid())?;
    let edges = usize::try_from(edges).map_err(|_| invalid())?;
    if count > causal_store::MAX_OWNER_EVENTS
        || bytes > causal_store::MAX_OWNER_BYTES
        || edges > causal_store::MAX_OWNER_DEPENDENCIES
    {
        return Err(invalid());
    }
    let (context_bytes, context_edges): (i64, i64) = sqlx::query_as("SELECT COALESCE(SUM(length(CAST(author_context AS BLOB))),0),COALESCE(SUM(json_array_length(author_context,'$.frontier')+json_array_length(author_context,'$.events')),0) FROM group_lifecycle_operations WHERE user_id=?")
        .bind(user.id).fetch_one(&mut *db).await?;
    if bytes.saturating_add(usize::try_from(context_bytes).map_err(|_| invalid())?)
        > causal_store::MAX_OWNER_BYTES
        || edges.saturating_add(usize::try_from(context_edges).map_err(|_| invalid())?)
            > causal_store::MAX_OWNER_DEPENDENCIES
    {
        return Err(invalid());
    }
    let wires: Vec<String> = sqlx::query_scalar("SELECT e.envelope FROM group_lifecycle_events m JOIN causal_events e USING(user_id,writer_id,counter) WHERE m.user_id=? AND e.encoding='portable_v2'")
        .bind(user.id).fetch_all(db).await?;
    wires
        .iter()
        .map(|wire| {
            let event = causal_envelope::parse_causal_envelope(wire).map_err(|_| invalid())?;
            if causal_envelope::canonical_causal_envelope(&event).map_err(|_| invalid())? != *wire {
                return Err(invalid());
            }

            Ok(event)
        })
        .collect()
}

pub(super) async fn write(
    db: &mut SqliteConnection,
    user: &User,
    operation: &str,
    focus: &str,
    events: &[CausalEnvelope],
    author_context: Option<&str>,
    remote: bool,
) -> Result<(), DbError> {
    sqlx::query("INSERT INTO group_lifecycle_operations(user_id,operation_id,focus_birth_id,event_count,author_context) VALUES(?,?,?,?,?)")
        .bind(user.id).bind(operation).bind(focus).bind(i64::try_from(events.len()).map_err(|_| invalid())?).bind(author_context).execute(&mut *db).await?;
    for (position, event) in events.iter().enumerate() {
        let wire = causal_envelope::canonical_causal_envelope(event).map_err(|_| invalid())?;
        if remote {
            causal_store::store_checked_receipt(db, user.id, event, &wire).await?;
        } else {
            sqlx::query(
                "INSERT INTO causal_events(user_id,writer_id,counter,envelope) VALUES(?,?,?,?)",
            )
            .bind(user.id)
            .bind(&event.event.writer_id)
            .bind(counter(&event.event)?)
            .bind(wire)
            .execute(&mut *db)
            .await?;
        }
        sqlx::query("INSERT INTO group_lifecycle_events(user_id,operation_id,position,writer_id,counter) VALUES(?,?,?,?,?)")
            .bind(user.id).bind(operation).bind(i64::try_from(position).map_err(|_| invalid())?).bind(&event.event.writer_id).bind(counter(&event.event)?).execute(&mut *db).await?;
    }
    if !remote {
        crate::sync_transport::operation_selection::record_local(
            db,
            user,
            operation,
            events.first().ok_or_else(invalid)?,
        )
        .await?;
    }
    // Limits include the entire operation, never a partially acknowledged prefix.
    events_check(db, user).await
}

async fn events_check(db: &mut SqliteConnection, user: &User) -> Result<(), DbError> {
    events(db, user).await?;

    Ok(())
}
