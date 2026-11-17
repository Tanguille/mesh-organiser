use sqlx::SqliteConnection;

use crate::{
    DbError,
    causal_envelope::{self, CausalEnvelope},
    db_context::DbContext,
    model::user::User,
    user_db::SyncIdentity,
    util::sha256_hex,
};

use super::{auth, failure};

#[derive(serde::Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Item {
    #[serde(rename = "group_identity_observation")]
    IdentityObservation {
        observation: crate::causal_genesis::identity::IndependentObservation,
    },
    #[serde(rename = "group_genesis")]
    Genesis {
        root: String,
        materialization: serde_json::Value,
    },
    #[serde(rename = "group_operation")]
    Operation {
        operation_id: String,
        focus_birth_id: String,
        envelopes: Vec<String>,
        #[serde(default)]
        author_context: Option<String>,
    },
}
impl Item {
    async fn apply(
        &self,
        db: &mut SqliteConnection,
        user: &User,
        identity: &SyncIdentity,
        source: &SyncIdentity,
        events: Vec<CausalEnvelope>,
    ) -> Result<(), DbError> {
        match self {
            Self::IdentityObservation { observation } => {
                crate::causal_genesis::identity::insert(db, user, observation, false).await?;
                crate::causal_group_lifecycle::project_identity_observation(db, user).await?;
            }
            Self::Genesis {
                root,
                materialization,
            } => {
                if materialization.as_object().is_none_or(|fields| {
                    fields.len() != 3
                        || !fields.contains_key("name")
                        || !fields.contains_key("created")
                        || !fields.contains_key("last_modified")
                }) {
                    return Err(failure("unsupported"));
                }
                crate::causal_genesis::transport::install_group(
                    db,
                    user,
                    identity,
                    source,
                    root,
                    materialization.clone(),
                )
                .await?;
            }
            Self::Operation {
                operation_id,
                focus_birth_id,
                author_context,
                ..
            } => {
                crate::causal_group_lifecycle::transport::apply(
                    db,
                    user,
                    operation_id,
                    focus_birth_id,
                    events,
                    author_context.as_deref(),
                )
                .await?;
            }
        }

        Ok(())
    }
    const fn kind(&self) -> &'static str {
        match self {
            Self::IdentityObservation { .. } => "group_identity_observation",
            Self::Genesis { .. } => "group_genesis",
            Self::Operation { .. } => "group_operation",
        }
    }
    fn wires(&self) -> Vec<&str> {
        match self {
            Self::IdentityObservation { observation } => vec![&observation.root],
            Self::Genesis { root, .. } => vec![root],
            Self::Operation { envelopes, .. } => envelopes.iter().map(String::as_str).collect(),
        }
    }
    fn parse(&self) -> Result<Vec<CausalEnvelope>, DbError> {
        let wires = self.wires();
        if wires.is_empty() || wires.len() > 1024 {
            return Err(failure("unsupported"));
        }
        wires
            .into_iter()
            .map(|wire| {
                let event = causal_envelope::parse_causal_envelope(wire)
                    .map_err(|_| failure("unsupported"))?;
                if causal_envelope::canonical_causal_envelope(&event)
                    .map_err(|_| failure("unsupported"))?
                    != wire
                {
                    return Err(failure("unsupported"));
                }

                Ok(event)
            })
            .collect()
    }
    fn id(&self, events: &[CausalEnvelope]) -> Result<String, DbError> {
        match self {
            Self::IdentityObservation { observation } => {
                observation.validate()?;
                // Populated roots require their complete batch and verified content.
                if observation.batch_id.is_some() {
                    return Err(failure("unsupported"));
                }
                Ok(observation.observation_id.clone())
            }
            Self::Genesis { .. } => events[0]
                .entity
                .birth_id
                .clone()
                .ok_or_else(|| failure("identity_blocked")),
            Self::Operation { operation_id, .. } if operation_id.len() == 32 => {
                Ok(operation_id.clone())
            }
            Self::Operation { .. } => Err(failure("unsupported")),
        }
    }
}
#[derive(Debug, serde::Serialize)]
pub struct Published {
    pub status: &'static str,
    pub sequence: String,
}
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageItem {
    pub sequence: String,
    pub digest: String,
    pub issuer: super::pull::Issuer,
    pub item: serde_json::Value,
}
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub server_identity: SyncIdentity,
    pub through: String,
    pub next_after: String,
    pub has_more: bool,
    pub items: Vec<PageItem>,
}

fn parse_item(
    value: serde_json::Value,
) -> Result<(Item, Vec<CausalEnvelope>, String, String), DbError> {
    let wire = serde_json::to_string(&value).map_err(|_| failure("unsupported"))?;
    if wire.len() > super::MAX_WIRE_ITEM_BYTES {
        return Err(failure("too_large"));
    }
    // Omission is the historical wire format; an explicit null would be lost
    // when the immutable log item is reconstructed for later pages.
    if value
        .get("author_context")
        .is_some_and(serde_json::Value::is_null)
    {
        return Err(failure("unsupported"));
    }
    let item: Item = serde_json::from_value(value).map_err(|_| failure("unsupported"))?;
    let events = item.parse()?;
    let id = item.id(&events)?;
    let digest = sha256_hex(wire.as_bytes());
    Ok((item, events, id, digest))
}

pub async fn publish(
    db: &DbContext,
    user: &User,
    account: &str,
    connection: &str,
    secret: &str,
    writer_nonce: Option<&str>,
    value: serde_json::Value,
) -> Result<Published, DbError> {
    let (item, events, id, digest) = parse_item(value)?;
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    let (identity, writer) =
        auth::authorize(&mut transaction, user, account, connection, secret).await?;
    let event_writer = match writer_nonce {
        Some(nonce) => super::offline::derived_writer(&identity, connection, nonce)?,
        None => writer.writer_id.clone(),
    };
    let prior: Option<(i64, String)> = sqlx::query_as(
        "SELECT sequence,digest FROM sync_transport_log WHERE user_id=? AND kind=? AND item_id=?",
    )
    .bind(user.id)
    .bind(item.kind())
    .bind(&id)
    .fetch_optional(&mut *transaction)
    .await?;
    if let Some((sequence, prior)) = prior {
        if prior != digest {
            return Err(failure("identity_blocked"));
        }
        transaction.commit().await?;
        return Ok(Published {
            status: "already_present",
            sequence: sequence.to_string(),
        });
    }
    // An authenticated account cannot impersonate another device's issued writer.
    // Relay is permitted only by the exact committed receipt above.
    if events
        .iter()
        .any(|event| event.event.writer_id != event_writer)
    {
        return Err(failure("forbidden"));
    }
    if let Some(nonce) = writer_nonce {
        super::offline::verify_delegation(
            &mut transaction,
            user,
            &identity,
            &writer.writer_id,
            &event_writer,
            nonce,
        )
        .await?;
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
        .bind(user.id)
        .fetch_one(&mut *transaction)
        .await?;
    if count >= 4096 {
        return Err(failure("quota_paused"));
    }
    item.apply(&mut transaction, user, &identity, &writer.source(), events)
        .await?;
    let sequence: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(sequence),0)+1 FROM sync_transport_log WHERE user_id=?",
    )
    .bind(user.id)
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query("INSERT INTO sync_transport_log(user_id,sequence,kind,item_id,digest,authority_writer_id,writer_id) VALUES(?,?,?,?,?,?,?)")
        .bind(user.id).bind(sequence).bind(item.kind()).bind(&id).bind(digest).bind(&writer.writer_id).bind(&event_writer).execute(&mut *transaction).await?;
    transaction.commit().await?;

    Ok(Published {
        status: "applied",
        sequence: sequence.to_string(),
    })
}

async fn item(
    db: &mut SqliteConnection,
    user: &User,
    kind: &str,
    id: &str,
) -> Result<serde_json::Value, DbError> {
    if kind == "group_identity_observation" {
        return crate::causal_genesis::identity::read(db, user, id).await;
    }
    if kind == "observed_group" {
        return crate::causal_genesis::populated::transport::read(db, user, id).await;
    }
    if kind == "group_genesis" {
        let (root,snapshot):(String,String)=sqlx::query_as("SELECT e.envelope,b.materialization FROM causal_genesis_bindings b JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.entity_kind='group' AND b.birth_id=? AND b.initialization_kind='creation'")
            .bind(user.id).bind(id).fetch_one(db).await?;
        let snapshot: serde_json::Value =
            serde_json::from_str(&snapshot).map_err(|_| failure("unsupported"))?;
        return Ok(
            serde_json::json!({"kind":"group_genesis","root":root,"materialization":{"name":snapshot["name"],"created":snapshot["created"],"last_modified":snapshot["last_modified"]}}),
        );
    }
    let (focus, context): (String, Option<String>) = sqlx::query_as(
        "SELECT focus_birth_id,author_context FROM group_lifecycle_operations WHERE user_id=? AND operation_id=?",
    )
    .bind(user.id)
    .bind(id)
    .fetch_one(&mut *db)
    .await?;
    let envelopes:Vec<String>=sqlx::query_scalar("SELECT c.envelope FROM group_lifecycle_events e JOIN causal_events c USING(user_id,writer_id,counter) WHERE e.user_id=? AND e.operation_id=? ORDER BY e.position").bind(user.id).bind(id).fetch_all(db).await?;

    let mut value = serde_json::json!({"kind":"group_operation","operation_id":id,"focus_birth_id":focus,"envelopes":envelopes});
    if let Some(context) = context {
        value["author_context"] = serde_json::Value::String(context);
    }

    Ok(value)
}

#[allow(clippy::too_many_arguments)]
pub async fn list(
    db: &DbContext,
    user: &User,
    account: &str,
    connection: &str,
    secret: &str,
    after: i64,
    through: Option<i64>,
    limit: usize,
) -> Result<Page, DbError> {
    if after < 0 || limit == 0 || limit > super::MAX_PAGE_ITEMS {
        return Err(failure("invalid_cursor"));
    }
    let mut transaction = db.begin().await?;
    let (identity, _) =
        auth::authorize(&mut transaction, user, account, connection, secret).await?;
    let high: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(sequence),0) FROM sync_transport_log WHERE user_id=?",
    )
    .bind(user.id)
    .fetch_one(&mut *transaction)
    .await?;
    let through = through.unwrap_or(high);
    if through < after || through > high {
        return Err(failure("invalid_cursor"));
    }
    let rows:Vec<(i64,String,String,String,String,String,String)>=sqlx::query_as("SELECT l.sequence,l.kind,l.item_id,l.digest,w.connection_id,l.authority_writer_id,l.writer_id FROM sync_transport_log l JOIN sync_transport_writers w ON w.user_id=l.user_id AND w.writer_id=l.authority_writer_id WHERE l.user_id=? AND l.sequence>? AND l.sequence<=? ORDER BY l.sequence LIMIT ?")
        .bind(user.id).bind(after).bind(through).bind(i64::try_from(limit).map_err(|_|failure("invalid_cursor"))?).fetch_all(&mut *transaction).await?;
    let mut page = Page {
        server_identity: identity,
        through: through.to_string(),
        next_after: after.to_string(),
        has_more: false,
        items: Vec::new(),
    };
    let mut page_bytes = 512;
    for (sequence, kind, id, digest, connection_id, authority_writer_id, writer_id) in rows {
        let value = item(&mut transaction, user, &kind, &id).await?;
        let wire = serde_json::to_string(&value).map_err(|_| failure("unsupported"))?;
        if sha256_hex(wire.as_bytes()) != digest {
            return Err(failure("unsupported"));
        }
        let row = PageItem {
            sequence: sequence.to_string(),
            digest,
            issuer: super::pull::Issuer {
                connection_id,
                authority_writer_id,
                writer_id,
            },
            item: value,
        };
        let bytes = serde_json::to_vec(&row)
            .map_err(|_| failure("unsupported"))?
            .len()
            + 1;
        if page_bytes + bytes > super::MAX_PAGE_BYTES {
            break;
        }
        page_bytes += bytes;
        page.next_after = sequence.to_string();
        page.items.push(row);
    }
    page.has_more = page.next_after != page.through;
    if page.items.is_empty() && page.has_more {
        return Err(failure("too_large"));
    }
    transaction.commit().await?;

    Ok(page)
}

/// Apply only a page delivered by the service's authenticated HTTPS client. This
/// internal DB API does not turn a caller-created identity into authentication.
pub async fn apply_delivery(
    db: &DbContext,
    delivery: super::pull::Delivery,
    value: serde_json::Value,
) -> Result<Published, DbError> {
    let (parsed, events, id, digest) = parse_item(value.clone())?;
    if digest != delivery.digest
        || events
            .iter()
            .any(|event| event.event.writer_id != delivery.issuer.writer_id)
    {
        return Err(failure("identity_blocked"));
    }
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    let replay = delivery
        .validate(&mut transaction, parsed.kind(), &id)
        .await?;
    if replay {
        transaction.commit().await?;
        return Ok(Published {
            status: "already_present",
            sequence: delivery.sequence.to_string(),
        });
    }
    let echo = if delivery
        .previously_applied(&mut transaction, parsed.kind(), &id)
        .await?
    {
        if item(
            &mut transaction,
            &delivery.authorization.user,
            parsed.kind(),
            &id,
        )
        .await?
            != value
        {
            return Err(failure("identity_blocked"));
        }
        true
    } else {
        source_echo(
            &mut transaction,
            &delivery.authorization,
            &delivery.issuer,
            &parsed,
            &events,
            &id,
            &value,
        )
        .await?
    };
    if !echo {
        parsed
            .apply(
                &mut transaction,
                &delivery.authorization.user,
                &delivery.authorization.identity,
                &delivery.authorization.issuer_origin(&delivery.issuer),
                events,
            )
            .await?;
    }
    delivery
        .commit(&mut transaction, parsed.kind(), &id)
        .await?;
    transaction.commit().await?;

    Ok(Published {
        status: if echo { "already_present" } else { "applied" },
        sequence: delivery.sequence.to_string(),
    })
}

pub(super) async fn verify_publication(
    db: &mut SqliteConnection,
    authorization: &super::pull::Authorization,
    issuer: &super::pull::Issuer,
    value: &serde_json::Value,
) -> Result<bool, DbError> {
    let (parsed, events, id, _) = parse_item(value.clone())?;
    if events
        .iter()
        .any(|event| event.event.writer_id != issuer.writer_id)
    {
        return Err(failure("unverified_author"));
    }

    source_echo(db, authorization, issuer, &parsed, &events, &id, value).await
}

async fn source_echo(
    db: &mut SqliteConnection,
    authorization: &super::pull::Authorization,
    issuer: &super::pull::Issuer,
    parsed: &Item,
    events: &[CausalEnvelope],
    id: &str,
    value: &serde_json::Value,
) -> Result<bool, DbError> {
    let owner = &authorization.user;
    let local = &authorization.identity;
    let server = &authorization.server;
    let delegated:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM local_writer_delegations WHERE user_id=? AND account_id=? AND writer_id=? AND authority_writer_id=? AND connection_id=? AND server_instance_id=? AND server_account_id=?) OR EXISTS(SELECT 1 FROM sync_transport_local_writers WHERE user_id=? AND local_instance_id=? AND local_account_id=? AND writer_id=? AND writer_id=? AND connection_id=? AND server_instance_id=? AND server_account_id=?)")
        .bind(owner.id).bind(&local.account_id).bind(&issuer.writer_id).bind(&issuer.authority_writer_id).bind(&issuer.connection_id).bind(&server.instance_id).bind(&server.account_id)
        .bind(owner.id).bind(&local.instance_id).bind(&local.account_id).bind(&issuer.writer_id).bind(&issuer.authority_writer_id).bind(&issuer.connection_id).bind(&server.instance_id).bind(&server.account_id).fetch_one(&mut *db).await?;
    if !delegated {
        return Ok(false);
    }
    for event in events {
        let local_event:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM causal_events e WHERE e.user_id=? AND e.writer_id=? AND e.counter=? AND e.encoding='portable_v2' AND NOT EXISTS(SELECT 1 FROM causal_event_receipts r WHERE r.user_id=e.user_id AND r.writer_id=e.writer_id AND r.counter=e.counter))")
            .bind(owner.id).bind(&event.event.writer_id).bind(event.event.counter.parse::<i64>().map_err(|_|failure("unsupported"))?).fetch_one(&mut *db).await?;
        if !local_event {
            return Ok(false);
        }
    }
    let present:bool=match parsed {
        Item::IdentityObservation{..}=>sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_identity_observations WHERE user_id=? AND observation_id=?)").bind(owner.id).bind(id).fetch_one(&mut *db).await?,
        Item::Genesis{..}=>sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM causal_genesis_bindings WHERE user_id=? AND entity_kind='group' AND birth_id=? AND initialization_kind='creation' AND origin='local' AND local_instance_id=? AND local_account_id=? AND source_instance_id=local_instance_id AND source_account_id=local_account_id)").bind(owner.id).bind(id).bind(&local.instance_id).bind(&local.account_id).fetch_one(&mut *db).await?,
        Item::Operation{..}=>sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_lifecycle_operations WHERE user_id=? AND operation_id=?)").bind(owner.id).bind(id).fetch_one(&mut *db).await?,
    };
    if !present {
        return Ok(false);
    }
    // Reconstruct immutable publication evidence, never the current local name,
    // destination or Trash projection, which may have changed since publication.
    if item(db, owner, parsed.kind(), id).await? != *value {
        return Err(failure("identity_blocked"));
    }

    Ok(true)
}
