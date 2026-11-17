//! Selected immutable publications, not an automatic scheduler or a second event authority.
use super::{failure, observations, publication::Publication, pull::Authorization};
use crate::{DbError, causal_envelope::Dot, db_context::DbContext};
use sqlx::SqliteConnection;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageFile {
    pub name: String,
    pub key: String,
    pub size: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageRecord {
    pub path: String,
    pub token: String,
    pub files: Vec<StageFile>,
}
pub struct Queued {
    pub id: String,
    pub item: serde_json::Value,
    pub event: Dot,
    pub ack_sequence: Option<String>,
    pub stage: Option<StageRecord>,
}

/// Retained work at this read snapshot; listed ids still need publication validation.
#[derive(Debug)]
pub struct PendingPublications {
    pub outbox_ids: Vec<String>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
    pub other_link_count: usize,
}

pub async fn pending(db: &DbContext, auth: &Authorization) -> Result<PendingPublications, DbError> {
    pending_page(db, auth, None).await
}

pub async fn pending_page(
    db: &DbContext,
    auth: &Authorization,
    cursor: Option<&str>,
) -> Result<PendingPublications, DbError> {
    let mut tx = db.begin().await?;
    auth.validate(&mut tx).await?;
    // This cursor is navigation, not authority: each read validates the current
    // account/link and scopes its query independently. It contains no secret.
    let scope = super::auth::digest(&format!(
        "mesh-outbox-page-v1:{}:{}",
        auth.user.id,
        auth.identity_choice_key()
    ));
    let after = if let Some(cursor) = cursor {
        let (owner, after) = cursor
            .split_once(':')
            .ok_or_else(|| failure("invalid_cursor"))?;
        if owner != scope || !super::auth::canonical_hex(after, 32) {
            return Err(failure("invalid_cursor"));
        }
        after
    } else {
        ""
    };
    // Immutable lexical ids remain a usable boundary even when its row is
    // discarded. New ids before this boundary require a fresh first-page read;
    // pagination is not a frozen snapshot or a claim that all work is complete.
    let mut ids: Vec<String> = sqlx::query_scalar("SELECT outbox_id FROM sync_publication_outbox WHERE user_id=? AND local_instance_id=? AND local_account_id=? AND server_instance_id=? AND server_account_id=? AND connection_id=? AND ack_sequence IS NULL AND outbox_id>? ORDER BY outbox_id LIMIT ?")
        .bind(auth.user.id).bind(&auth.identity.instance_id).bind(&auth.identity.account_id).bind(&auth.server.instance_id).bind(&auth.server.account_id).bind(&auth.connection)
        .bind(after)
        .bind(i64::try_from(super::MAX_PAGE_ITEMS + 1).map_err(|_| failure("too_large"))?).fetch_all(&mut *tx).await?;
    // Other-link work remains visible as blocked work. Reconnecting must never
    // silently rebind an old publication or claim that this account has no queue.
    let other: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_publication_outbox WHERE user_id=? AND ack_sequence IS NULL AND NOT(local_instance_id=? AND local_account_id=? AND server_instance_id=? AND server_account_id=? AND connection_id=?)")
        .bind(auth.user.id).bind(&auth.identity.instance_id).bind(&auth.identity.account_id).bind(&auth.server.instance_id).bind(&auth.server.account_id).bind(&auth.connection).fetch_one(&mut *tx).await?;
    let has_more = ids.len() > super::MAX_PAGE_ITEMS;
    ids.truncate(super::MAX_PAGE_ITEMS);
    let next_cursor = if has_more {
        let last = ids.last().ok_or_else(|| failure("invalid_cursor"))?;
        if !super::auth::canonical_hex(last, 32) {
            return Err(failure("invalid_cursor"));
        }
        Some(format!("{scope}:{last}"))
    } else {
        None
    };
    tx.commit().await?;

    Ok(PendingPublications {
        outbox_ids: ids,
        has_more,
        next_cursor,
        other_link_count: usize::try_from(other).map_err(|_| failure("too_large"))?,
    })
}

fn item_identity(item: &serde_json::Value) -> Result<(&str, String), DbError> {
    let kind = item
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| failure("unsupported"))?;
    let id = match kind {
        "observed_group" => item
            .get("batch_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        "group_identity_observation" => item
            .get("observation")
            .and_then(|value| value.get("observation_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        "group_operation" => item
            .get("operation_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        "group_genesis" => item
            .get("root")
            .and_then(serde_json::Value::as_str)
            .and_then(|wire| crate::causal_envelope::parse_causal_envelope(wire).ok())
            .and_then(|root| root.entity.birth_id),
        _ => None,
    }
    .ok_or_else(|| failure("unsupported"))?;
    Ok((kind, id))
}

pub async fn queue(
    db: &DbContext,
    auth: &Authorization,
    event: &Dot,
    item: &serde_json::Value,
) -> Result<String, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let id = queue_in(&mut tx, auth, event, item).await?;
    tx.commit().await?;
    Ok(id)
}

/// Enqueue an already certified original in the domain mutation's transaction.
pub(crate) async fn queue_in(
    db: &mut SqliteConnection,
    auth: &Authorization,
    event: &Dot,
    item: &serde_json::Value,
) -> Result<String, DbError> {
    Ok(insert(db, auth, event, item, None).await?.0)
}

pub async fn queue_observation(
    db: &DbContext,
    auth: &Authorization,
    group: i64,
    capture: Option<&crate::causal_adoption::PopulatedGroupObservation>,
    contents: &[observations::BlobContent],
    stage: &StageRecord,
) -> Result<(String, bool), DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    auth.validate(&mut tx).await?;
    let bundle = if let Some(capture) = capture {
        crate::causal_genesis::populated::transport::enroll_source_in(
            &mut tx,
            &auth.user,
            &auth.identity,
            capture,
            contents,
        )
        .await?
    } else {
        crate::causal_genesis::populated::transport::reload_source(
            &mut tx,
            &auth.user,
            &auth.identity,
            group,
        )
        .await?
        .ok_or_else(|| failure("identity_blocked"))?
    };
    let result = queue_enrolled_observation_in(&mut tx, auth, &bundle, contents, stage).await?;
    tx.commit().await?;
    Ok(result)
}

pub(crate) async fn queue_enrolled_observation_in(
    db: &mut SqliteConnection,
    auth: &Authorization,
    bundle: &crate::causal_adoption::ObservedPopulatedGroupBundle,
    contents: &[observations::BlobContent],
    stage: &StageRecord,
) -> Result<(String, bool), DbError> {
    let item = observations::encode(bundle, contents.to_vec())?;

    insert(db, auth, bundle.group_event(), &item, Some(stage)).await
}

async fn insert(
    db: &mut SqliteConnection,
    auth: &Authorization,
    event: &Dot,
    item: &serde_json::Value,
    stage: Option<&StageRecord>,
) -> Result<(String, bool), DbError> {
    let proof = Publication::prepare_in(db, auth, event, item).await?;
    let (kind, item_id) = item_identity(item)?;
    let prior: Option<(String,String,String)> = sqlx::query_as("SELECT outbox_id,digest,connection_id FROM sync_publication_outbox WHERE user_id=? AND server_instance_id=? AND server_account_id=? AND kind=? AND item_id=?")
        .bind(auth.user.id).bind(&auth.server.instance_id).bind(&auth.server.account_id).bind(kind).bind(&item_id).fetch_optional(&mut *db).await?;
    if let Some((id, digest, connection)) = prior {
        if digest != proof.digest || connection != auth.connection {
            return Err(failure("identity_blocked"));
        }
        if kind == "group_operation" {
            super::operation_selection::selected_in(db, auth, &item_id, &id).await?;
        }
        return Ok((id, false));
    }
    let wire = serde_json::to_string(item).map_err(|_| failure("unsupported"))?;
    let request = serde_json::to_string(&serde_json::json!({"protocol":1,"connection_id":auth.connection,"writer_nonce":proof.nonce,"item":item})).map_err(|_| failure("unsupported"))?;
    if request.len() > super::MAX_WIRE_ITEM_BYTES {
        return Err(failure("too_large"));
    }
    let (files, logical_bytes) = validate_stage(db, kind, item, stage).await?;
    let (count, bytes, pending): (i64,i64,i64) = sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(length(CAST(o.item AS BLOB))+COALESCE(length(CAST(s.files AS BLOB)),0)+512),0),COALESCE(SUM(s.logical_bytes),0) FROM sync_publication_outbox o LEFT JOIN sync_publication_stages s USING(outbox_id) WHERE o.user_id=? AND o.ack_sequence IS NULL")
        .bind(auth.user.id).fetch_one(&mut *db).await?;
    if count
        >= i64::try_from(crate::causal_store::MAX_OWNER_EVENTS).map_err(|_| failure("too_large"))?
        || usize::try_from(bytes)
            .ok()
            .and_then(|bytes| bytes.checked_add(wire.len() + files.len() + 512))
            .is_none_or(|total| total > crate::causal_store::MAX_OWNER_BYTES)
        || u64::try_from(pending)
            .ok()
            .and_then(|bytes| bytes.checked_add(logical_bytes))
            .is_none_or(|total| total > observations::MAX_TOTAL_BYTES)
    {
        return Err(failure("quota_paused"));
    }
    let id = crate::random_hex_32();
    sqlx::query("INSERT INTO sync_publication_outbox(outbox_id,user_id,local_instance_id,local_account_id,server_instance_id,server_account_id,connection_id,authority_writer_id,writer_id,writer_nonce,event_counter,kind,item_id,digest,item) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(&id).bind(auth.user.id).bind(&auth.identity.instance_id).bind(&auth.identity.account_id).bind(&auth.server.instance_id).bind(&auth.server.account_id).bind(&proof.issuer.connection_id).bind(&proof.issuer.authority_writer_id).bind(&proof.issuer.writer_id).bind(&proof.nonce).bind(&event.counter).bind(kind).bind(&item_id).bind(proof.digest).bind(wire).execute(&mut *db).await?;
    if kind == "group_operation" {
        super::operation_selection::selected_in(db, auth, &item_id, &id).await?;
    }
    if let Some(stage) = stage {
        sqlx::query("INSERT INTO sync_publication_stages(outbox_id,stage_path,token,files,logical_bytes) VALUES(?,?,?,?,?)")
            .bind(&id).bind(&stage.path).bind(&stage.token).bind(files).bind(i64::try_from(logical_bytes).map_err(|_| failure("too_large"))?).execute(db).await?;
    }
    Ok((id, stage.is_some()))
}

async fn validate_stage(
    db: &mut SqliteConnection,
    kind: &str,
    item: &serde_json::Value,
    stage: Option<&StageRecord>,
) -> Result<(String, u64), DbError> {
    let Some(stage) = stage else {
        // Empty observed groups have no file lifetime to certify. Parsing still
        // requires a complete manifest, so a nonempty batch cannot use this path.
        if kind == "observed_group" && !observations::parse(item)?.contents().is_empty() {
            return Err(failure("unsupported"));
        }
        return Ok((String::new(), 0));
    };
    if kind != "observed_group" {
        return Err(failure("unsupported"));
    }
    let certified: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_transfer_intents WHERE stage_path=? AND token=? AND manifest='[]')")
        .bind(&stage.path).bind(&stage.token).fetch_one(db).await?;
    let plan = observations::parse(item)?;
    if !certified || stage.files.len() != plan.contents().len() {
        return Err(failure("identity_blocked"));
    }
    let mut keys = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    let mut total = 0_u64;
    for file in &stage.files {
        if file
            .name
            .parse::<usize>()
            .ok()
            .is_none_or(|n| n.to_string() != file.name)
            || !keys.insert(&file.key)
            || !names.insert(&file.name)
            || !plan.contents().iter().any(|content| {
                content.sha256 == file.key && u64::try_from(content.size) == Ok(file.size)
            })
        {
            return Err(failure("identity_blocked"));
        }
        total = total
            .checked_add(file.size)
            .ok_or_else(|| failure("too_large"))?;
    }
    Ok((
        serde_json::to_string(&stage.files).map_err(|_| failure("unsupported"))?,
        total,
    ))
}

pub async fn selected_observation(
    db: &DbContext,
    auth: &Authorization,
    group: i64,
) -> Result<Option<String>, DbError> {
    let mut tx = db.begin().await?;
    auth.validate(&mut tx).await?;
    let id = sqlx::query_scalar("SELECT o.outbox_id FROM models_group g JOIN causal_genesis_bindings b ON b.local_id=g.group_id AND b.user_id=g.group_user_id AND b.entity_kind='group' AND b.birth_id=g.group_birth_id AND b.global_id=g.group_unique_global_id JOIN causal_observation_batch_members m USING(user_id,entity_kind,birth_id) JOIN sync_publication_outbox o ON o.user_id=b.user_id AND o.kind='observed_group' AND o.item_id=m.batch_id WHERE g.group_id=? AND g.group_user_id=? AND o.local_instance_id=? AND o.local_account_id=? AND o.server_instance_id=? AND o.server_account_id=? AND o.connection_id=?")
        .bind(group).bind(auth.user.id).bind(&auth.identity.instance_id).bind(&auth.identity.account_id).bind(&auth.server.instance_id).bind(&auth.server.account_id).bind(&auth.connection).fetch_optional(&mut *tx).await?;
    tx.commit().await?;
    Ok(id)
}

/// A durable decision survives explicit discard, but cannot promise queued work
/// after the user removed its publication. Acknowledged rows remain valid receipts.
pub(crate) async fn require_retained(
    db: &mut SqliteConnection,
    auth: &Authorization,
    id: &str,
    event: &Dot,
) -> Result<(), DbError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sync_publication_outbox WHERE outbox_id=? AND user_id=?)",
    )
    .bind(id)
    .bind(auth.user.id)
    .fetch_one(&mut *db)
    .await?;
    if !exists {
        return Err(failure("publication_discarded"));
    }
    let queued = load_in(db, auth, id).await?;
    if queued.event != *event {
        return Err(failure("identity_blocked"));
    }
    Ok(())
}

pub async fn load(db: &DbContext, auth: &Authorization, id: &str) -> Result<Queued, DbError> {
    let mut tx = db.begin().await?;
    let result = load_in(&mut tx, auth, id).await?;
    tx.commit().await?;
    Ok(result)
}
async fn load_in(
    db: &mut SqliteConnection,
    auth: &Authorization,
    id: &str,
) -> Result<Queued, DbError> {
    auth.validate(db).await?;
    let (wire,writer,counter,digest,nonce,authority,ack_sequence): (Option<String>,String,String,String,String,String,Option<String>) = sqlx::query_as("SELECT item,writer_id,event_counter,digest,writer_nonce,authority_writer_id,ack_sequence FROM sync_publication_outbox WHERE outbox_id=? AND user_id=? AND local_instance_id=? AND local_account_id=? AND server_instance_id=? AND server_account_id=? AND connection_id=?")
        .bind(id).bind(auth.user.id).bind(&auth.identity.instance_id).bind(&auth.identity.account_id).bind(&auth.server.instance_id).bind(&auth.server.account_id).bind(&auth.connection).fetch_one(&mut *db).await?;
    let event = Dot {
        writer_id: writer,
        counter,
    };
    let item = if let Some(sequence) = &ack_sequence {
        if wire.is_some()
            || sequence
                .parse::<i64>()
                .ok()
                .is_none_or(|n| n <= 0 || n.to_string() != *sequence)
            || !super::auth::canonical_hex(&digest, 64)
        {
            return Err(failure("identity_blocked"));
        }
        let (original_nonce, issuer) = Publication::proof_in(db, auth, &event).await?;
        if original_nonce != nonce || issuer.authority_writer_id != authority {
            return Err(failure("identity_blocked"));
        }
        serde_json::Value::Null
    } else {
        let item =
            serde_json::from_str(wire.as_deref().ok_or_else(|| failure("identity_blocked"))?)
                .map_err(|_| failure("unsupported"))?;
        let proof = Publication::prepare_in(db, auth, &event, &item).await?;
        if proof.digest != digest
            || proof.nonce != nonce
            || proof.issuer.authority_writer_id != authority
        {
            return Err(failure("identity_blocked"));
        }
        item
    };
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT stage_path,token,files FROM sync_publication_stages WHERE outbox_id=?",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    let stage = row
        .map(|(path, token, files)| {
            Ok::<_, DbError>(StageRecord {
                path,
                token,
                files: serde_json::from_str(&files).map_err(|_| failure("unsupported"))?,
            })
        })
        .transpose()?;
    Ok(Queued {
        id: id.into(),
        item,
        event,
        ack_sequence,
        stage,
    })
}

pub async fn acknowledge(
    db: &DbContext,
    auth: &Authorization,
    id: &str,
    sequence: &str,
) -> Result<(), DbError> {
    if sequence
        .parse::<i64>()
        .ok()
        .is_none_or(|n| n <= 0 || n.to_string() != sequence)
    {
        return Err(failure("invalid_cursor"));
    }
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let queued = load_in(&mut tx, auth, id).await?;
    if let Some(prior) = queued.ack_sequence {
        if prior != sequence {
            return Err(failure("identity_blocked"));
        }
    } else {
        sqlx::query("UPDATE sync_publication_outbox SET ack_sequence=?,item=NULL WHERE outbox_id=? AND ack_sequence IS NULL").bind(sequence).bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM sync_publication_stages WHERE outbox_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn stage_retained(db: &DbContext, path: &str, token: &str) -> Result<bool, DbError> {
    // A cancelled caller can release its lease while SQLite is still completing
    // COMMIT. Serialize with that writer before authorizing filesystem cleanup.
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let retained = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sync_publication_stages WHERE stage_path=? AND token=?)",
    )
    .bind(path)
    .bind(token)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(retained)
}

async fn discard_stage_in(
    db: &mut SqliteConnection,
    user: &crate::model::user::User,
    identity: &crate::user_db::SyncIdentity,
    id: &str,
) -> Result<Option<StageRecord>, DbError> {
    crate::sync_peer::validate_local(db, user, identity).await?;
    let owner:Option<(i64,String,String,Option<String>)> = sqlx::query_as("SELECT user_id,local_instance_id,local_account_id,ack_sequence FROM sync_publication_outbox WHERE outbox_id=?").bind(id).fetch_optional(&mut *db).await?;
    let Some((owner, instance, account, ack)) = owner else {
        return Ok(None);
    };
    if owner != user.id
        || instance != identity.instance_id
        || account != identity.account_id
        || ack.is_some()
    {
        return Err(failure("identity_blocked"));
    }
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT stage_path,token,files FROM sync_publication_stages WHERE outbox_id=?",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    row.map(|(path, token, files)| {
        Ok(StageRecord {
            path,
            token,
            files: serde_json::from_str(&files).map_err(|_| failure("unsupported"))?,
        })
    })
    .transpose()
}

pub async fn discard_stage(
    db: &DbContext,
    user: &crate::model::user::User,
    identity: &crate::user_db::SyncIdentity,
    id: &str,
) -> Result<Option<StageRecord>, DbError> {
    let mut tx = db.begin().await?;
    let result = discard_stage_in(&mut tx, user, identity, id).await?;
    tx.commit().await?;
    Ok(result)
}

/// Explicit cancellation preserves the original journal and never fabricates an ACK.
pub async fn discard(
    db: &DbContext,
    user: &crate::model::user::User,
    identity: &crate::user_db::SyncIdentity,
    id: &str,
    expected: Option<&StageRecord>,
) -> Result<(), DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    if discard_stage_in(&mut tx, user, identity, id)
        .await?
        .as_ref()
        != expected
    {
        return Err(failure("identity_blocked"));
    }
    sqlx::query("DELETE FROM sync_publication_outbox WHERE outbox_id=? AND user_id=? AND ack_sequence IS NULL").bind(id).bind(user.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
