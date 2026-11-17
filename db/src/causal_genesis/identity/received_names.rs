//! Receiving devices need an authenticated offer, independently of the source's identity decision.
use super::{
    Authorization, DbContext, DbError, Dot, Entity, NamePublication, ReceivedNameChoice, encode,
    invalid, names, observation, populated,
};
use crate::causal_group_rename::{NameState, NameToken};
use sqlx::SqliteConnection;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Token {
    entity: Entity,
    generation: i64,
    revision: i64,
    frontier: Vec<Dot>,
}
impl From<&NameToken> for Token {
    fn from(token: &NameToken) -> Self {
        Self {
            entity: token.entity.clone(),
            generation: token.generation,
            revision: token.revision,
            frontier: token.frontier.clone(),
        }
    }
}
impl From<Token> for NameToken {
    fn from(token: Token) -> Self {
        Self {
            entity: token.entity,
            generation: token.generation,
            revision: token.revision,
            frontier: token.frontier,
        }
    }
}

#[derive(sqlx::FromRow, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Evidence {
    observation_id: String,
    stream_id: String,
    sequence: i64,
    kind: String,
    item_id: String,
    digest: String,
    issuer_connection_id: String,
    authority_writer_id: String,
    writer_id: String,
}

// Reconstruct the original item and compare its digest with the actual applied
// HTTPS receipt. A local echo or an in-process import alone grants no offer.
async fn evidence(
    db: &mut SqliteConnection,
    auth: &Authorization,
    entity: &Entity,
    observation_id: Option<&str>,
) -> Result<Evidence, DbError> {
    let rows: Vec<Evidence> = sqlx::query_as("SELECT o.observation_id,r.stream_id,r.sequence,r.kind,r.item_id,r.digest,r.issuer_connection_id,r.authority_writer_id,r.writer_id FROM group_identity_observations o JOIN causal_event_receipts c ON c.user_id=o.user_id AND c.writer_id=o.writer_id AND c.counter=o.counter JOIN sync_pull_receipts r ON r.kind=CASE WHEN o.batch_id IS NULL THEN 'group_identity_observation' ELSE 'observed_group' END AND r.item_id=COALESCE(o.batch_id,o.observation_id) AND r.writer_id=o.writer_id JOIN sync_pull_streams s USING(stream_id) WHERE o.user_id=? AND o.birth_id=? AND s.user_id=o.user_id AND s.origin=? AND s.server_instance_id=? AND s.server_account_id=? AND s.last_applied>=r.sequence AND (? IS NULL OR o.observation_id=?) ORDER BY o.observation_id LIMIT 4097")
        .bind(auth.user.id).bind(&entity.birth_id).bind(&auth.origin).bind(&auth.server.instance_id).bind(&auth.server.account_id).bind(observation_id).bind(observation_id).fetch_all(&mut *db).await?;
    if rows.len() > crate::causal_store::MAX_OWNER_EVENTS {
        return Err(invalid());
    }
    let row = rows.into_iter().next().ok_or_else(invalid)?;
    let original = super::read(db, &auth.user, &row.observation_id).await?;
    let value: super::IndependentObservation =
        serde_json::from_value(original["observation"].clone()).map_err(|_| invalid())?;
    let (root, _) = value.validate()?;
    if root.entity != *entity || root.event.writer_id != row.writer_id {
        return Err(invalid());
    }
    let item = if let Some(batch) = value.batch_id {
        populated::transport::read(db, &auth.user, &batch).await?
    } else {
        original
    };
    if observation::digest(&encode(&item)?) != row.digest {
        return Err(invalid());
    }

    Ok(row)
}

pub async fn prepare_received_name(
    db: &DbContext,
    auth: &Authorization,
    group_id: i64,
) -> Result<ReceivedNameChoice, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    auth.validate(&mut tx).await?;
    let Some(NameState::Conflict { choices, token }) =
        crate::causal_group_lifecycle::name_state(&mut tx, &auth.user, group_id).await?
    else {
        return Err(invalid());
    };
    let (current_id, state) = names::current(
        &mut tx,
        auth,
        token.entity.birth_id.as_deref().ok_or_else(invalid)?,
    )
    .await?;
    if current_id != Some(group_id) || !matches!(state, NameState::Conflict { .. }) {
        return Err(invalid());
    }
    let receipt = evidence(&mut tx, auth, &token.entity, None).await?;
    let token_text = encode(&Token::from(&token))?;
    let receipt_text = encode(&receipt)?;
    let authorization = auth.identity_choice_key();
    let prior: Option<String> = sqlx::query_scalar("SELECT choice_id FROM received_group_name_choices WHERE user_id=? AND authorization=? AND birth_id=? AND token=? AND evidence=? AND selected_name IS NULL")
        .bind(auth.user.id).bind(&authorization).bind(&token.entity.birth_id).bind(&token_text).bind(&receipt_text).fetch_optional(&mut *tx).await?;
    let choice_id = if let Some(prior) = prior {
        prior
    } else {
        let (count, bytes): (i64,i64) = sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(length(CAST(token AS BLOB))+length(CAST(evidence AS BLOB))),0) FROM received_group_name_choices WHERE user_id=?").bind(auth.user.id).fetch_one(&mut *tx).await?;
        if usize::try_from(count).map_err(|_| invalid())? >= crate::causal_store::MAX_OWNER_EVENTS
            || usize::try_from(bytes)
                .map_err(|_| invalid())?
                .saturating_add(token_text.len())
                .saturating_add(receipt_text.len())
                > crate::causal_store::MAX_OWNER_BYTES
        {
            return Err(invalid());
        }
        let id = crate::random_hex_32();
        sqlx::query("INSERT INTO received_group_name_choices(choice_id,user_id,authorization,birth_id,token,evidence) VALUES(?,?,?,?,?,?)")
            .bind(&id).bind(auth.user.id).bind(authorization).bind(&token.entity.birth_id).bind(token_text).bind(receipt_text).execute(&mut *tx).await?;
        id
    };
    tx.commit().await?;

    Ok(ReceivedNameChoice { choice_id, choices })
}

#[derive(sqlx::FromRow)]
struct Offer {
    birth_id: String,
    token: String,
    evidence: String,
    selected_name: Option<String>,
    writer_id: Option<String>,
    counter: Option<i64>,
    outbox_id: Option<String>,
}

pub async fn resolve_received_name(
    db: &DbContext,
    auth: &Authorization,
    choice_id: &str,
    name: &str,
) -> Result<NamePublication, DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let result = resolve_in(&mut tx, auth, choice_id, name).await?;
    tx.commit().await?;

    Ok(result)
}

async fn resolve_in(
    db: &mut SqliteConnection,
    auth: &Authorization,
    choice_id: &str,
    name: &str,
) -> Result<NamePublication, DbError> {
    auth.validate(db).await?;
    let offer: Offer = sqlx::query_as("SELECT birth_id,token,evidence,selected_name,writer_id,counter,outbox_id FROM received_group_name_choices WHERE choice_id=? AND user_id=? AND authorization=?")
        .bind(choice_id).bind(auth.user.id).bind(auth.identity_choice_key()).fetch_optional(&mut *db).await?.ok_or_else(invalid)?;
    let token: Token = serde_json::from_str(&offer.token).map_err(|_| invalid())?;
    let token = NameToken::from(token);
    if token.entity.birth_id.as_deref() != Some(offer.birth_id.as_str()) {
        return Err(invalid());
    }
    let receipt: Evidence = serde_json::from_str(&offer.evidence).map_err(|_| invalid())?;
    if evidence(db, auth, &token.entity, Some(&receipt.observation_id)).await? != receipt {
        return Err(invalid());
    }
    let (group_id, state) = names::current(db, auth, &offer.birth_id).await?;
    if let Some(selected) = offer.selected_name {
        if selected != name {
            return Err(invalid());
        }
        let event = Dot {
            writer_id: offer.writer_id.ok_or_else(invalid)?,
            counter: offer.counter.ok_or_else(invalid)?.to_string(),
        };
        let outbox_id = offer.outbox_id.ok_or_else(invalid)?;
        crate::sync_transport::outbox::require_retained(db, auth, &outbox_id, &event).await?;

        return Ok(NamePublication::Queued {
            outbox_id,
            event,
            already_present: true,
            state,
        });
    }
    let result = names::resolve_and_queue(db, auth, group_id, state, &token, name).await?;
    if let NamePublication::Queued {
        outbox_id, event, ..
    } = &result
    {
        sqlx::query("UPDATE received_group_name_choices SET selected_name=?,writer_id=?,counter=?,outbox_id=? WHERE choice_id=? AND user_id=? AND selected_name IS NULL")
            .bind(name).bind(&event.writer_id).bind(super::super::counter(event)?).bind(outbox_id).bind(choice_id).bind(auth.user.id).execute(db).await?;
    }

    Ok(result)
}
