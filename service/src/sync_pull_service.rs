//! Experimental HTTPS-bound pull. No desktop coordinator or production route enables it.
use std::time::Duration;

pub use coordinator::{AdvanceOutcome, OperationSelection};
pub use identity_choice::{
    GroupIdentityChoice, GroupIdentityOutcome, GroupSeparationOutcome, NamePublication,
    ReceivedNameChoice,
};
pub use model_identity::{
    GroupIdentityDecision, ModelIdentityChoice, ModelIdentityDecision, ModelIdentityOutcome,
};

pub use db::sync_transport::outbox::PendingPublications;
use db::{
    model::user::User,
    sync_transport::{
        self,
        pull::{Authorization, Delivery},
    },
    user_db::SyncIdentity,
};

use crate::{AppState, ServiceError, sync_observation_service};
mod coordinator;
mod identity_choice;
mod model_identity;
mod outbox;
mod publication;
#[derive(Debug)]
pub enum HubTrust {
    System,
    AdditionalCaPem(Vec<u8>),
}
#[derive(Debug, PartialEq, Eq)]
pub enum PullOutcome {
    Applied { sequence: String },
    AlreadyApplied { sequence: String },
    Idle { through: String },
}
#[derive(Debug, PartialEq, Eq)]
pub enum PublicationOutcome {
    Published {
        sequence: String,
    },
    AlreadyPresent {
        sequence: String,
    },
    NeedsDependencies {
        missing: Vec<db::causal_envelope::Dependency>,
    },
}
enum NextDelivery {
    Item(Box<Delivery>, serde_json::Value),
    Idle(String),
}
pub struct AuthenticatedHub {
    client: reqwest::Client,
    authorization: Authorization,
}
/// Explicitly discard a selected pending item; this cannot retract a remote commit.
pub async fn discard_queued_publication(
    state: &AppState,
    user: &User,
    identity: &SyncIdentity,
    outbox_id: &str,
) -> Result<(), ServiceError> {
    sync_observation_service::discard_queued(state, user, identity, outbox_id).await
}
impl std::fmt::Debug for AuthenticatedHub {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthenticatedHub")
            .field("origin", &self.authorization.origin)
            .finish_non_exhaustive()
    }
}
fn invalid() -> ServiceError {
    db::DbError::Protocol("sync_transport:invalid_delivery".into()).into()
}
fn cursor(value: &str) -> Result<i64, ServiceError> {
    let number = value.parse::<i64>().map_err(|_| invalid())?;
    if number < 0 || number.to_string() != value {
        return Err(invalid());
    }

    Ok(number)
}
async fn json<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
    max: usize,
) -> Result<T, ServiceError> {
    if response.status() != reqwest::StatusCode::OK {
        return Err(db::DbError::Protocol(format!(
            "sync_transport:http_status:{}",
            response.status().as_u16()
        ))
        .into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > max {
            return Err(invalid());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| db::DbError::Protocol("sync_transport:invalid_response_shape".into()).into())
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    protocol: u32,
    server_identity: SyncIdentity,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    server_identity: SyncIdentity,
    connection_id: String,
    writer_id: String,
    writer_secret: String,
}
impl From<Registration> for sync_transport::WriterRegistration {
    fn from(value: Registration) -> Self {
        Self {
            server_identity: value.server_identity,
            connection_id: value.connection_id,
            writer_id: value.writer_id,
            writer_secret: value.writer_secret,
        }
    }
}
async fn authenticated_client(
    origin: &str,
    login_token: &str,
    trust: HubTrust,
) -> Result<(reqwest::Client, SyncIdentity), ServiceError> {
    let url = reqwest::Url::parse(origin).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || url.origin().ascii_serialization() != origin
    {
        return Err(invalid());
    }
    let mut builder = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .cookie_store(true)
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10));
    if let HubTrust::AdditionalCaPem(pem) = trust {
        builder = builder.add_root_certificate(reqwest::Certificate::from_pem(&pem)?);
    }
    let client = builder.build()?;
    let response = client
        .post(format!("{origin}/api/v1/login/token"))
        .json(&serde_json::json!({"token":login_token}))
        .send()
        .await?;
    if response.status() != reqwest::StatusCode::NO_CONTENT {
        return Err(invalid());
    }
    let probe: Identity = json(
        client
            .get(format!("{origin}/api/v2/sync/identity?protocol=1"))
            .send()
            .await?,
        4096,
    )
    .await?;
    if probe.protocol != 1 {
        return Err(invalid());
    }

    Ok((client, probe.server_identity))
}
impl AuthenticatedHub {
    /// Link an explicitly selected hub after authenticating its HTTPS session.
    /// This does not enable the production sync coordinator or mount any route.
    pub async fn pair(
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        origin: &str,
        login_token: &str,
        trust: HubTrust,
    ) -> Result<Self, ServiceError> {
        let intent = sync_transport::pairing::Pairing::capture(&state.db, user, identity).await?;
        let (client, server) = authenticated_client(origin, login_token, trust).await?;
        let registration = if let Some(existing) = intent.registration() {
            if existing.server_identity != server {
                return Err(
                    db::DbError::Protocol("sync_transport:server_identity_changed".into()).into(),
                );
            }
            existing.clone()
        } else {
            let registration: Registration = json(
                client
                    .post(format!("{origin}/api/v2/sync/handshake"))
                    .json(&serde_json::json!({"protocol":1,"client_identity":identity}))
                    .send()
                    .await?,
                4096,
            )
            .await?;
            if registration.server_identity != server {
                return Err(
                    db::DbError::Protocol("sync_transport:server_identity_changed".into()).into(),
                );
            }
            registration.into()
        };
        // Token login proves the account, not continued validity of its device
        // grant. An empty bounded page authenticates the grant without applying
        // history or advancing any local cursor.
        let probe: sync_transport::Page = json(
            client
                .get(format!("{origin}/api/v2/sync/items"))
                .header("x-sync-writer-secret", &registration.writer_secret)
                .query(&[
                    ("protocol", "1"),
                    ("connection_id", registration.connection_id.as_str()),
                    ("after", "0"),
                    ("through", "0"),
                    ("limit", "1"),
                ])
                .send()
                .await?,
            4096,
        )
        .await?;
        if probe.server_identity != server
            || probe.through != "0"
            || probe.next_after != "0"
            || probe.has_more
            || !probe.items.is_empty()
        {
            return Err(invalid());
        }
        // A cancelled request may leave an unused bounded server grant. Local
        // configuration is committed atomically, and a completed retry reuses it.
        let db = state.db.clone();
        let origin = origin.to_owned();
        let token = login_token.to_owned();
        let authorization =
            tokio::spawn(async move { intent.commit(&db, &origin, &token, &registration).await })
                .await
                .map_err(|_| invalid())??;

        Ok(Self {
            client,
            authorization,
        })
    }
    pub async fn connect(
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        origin: &str,
        login_token: &str,
        trust: HubTrust,
    ) -> Result<Self, ServiceError> {
        let authorization =
            Authorization::capture(&state.db, user, identity, origin, login_token).await?;
        let (client, server) = authenticated_client(origin, login_token, trust).await?;
        if server != authorization.server {
            return Err(
                db::DbError::Protocol("sync_transport:server_identity_changed".into()).into(),
            );
        }
        authorization.position(&state.db).await?;

        Ok(Self {
            client,
            authorization,
        })
    }
    pub async fn pull_next(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
    ) -> Result<PullOutcome, ServiceError> {
        tokio::time::timeout(Duration::from_secs(120), self.pull(state, user, identity))
            .await
            .map_err(|_| invalid())?
    }
    async fn next_delivery(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
    ) -> Result<NextDelivery, ServiceError> {
        // Capture again binds method arguments to the authenticated local account and link.
        let current = db::user_db::get_sync_identity(&state.db, user.id).await?;
        if !self.authorization.matches(user, identity) || current.as_ref() != Some(identity) {
            return Err(invalid());
        }
        let (stream, after, through) = self.authorization.position(&state.db).await?;
        let mut request = self
            .client
            .get(format!("{}/api/v2/sync/items", self.authorization.origin))
            .header("x-sync-writer-secret", self.authorization.secret())
            .query(&[
                ("protocol", "1"),
                ("connection_id", self.authorization.connection.as_str()),
                ("after", &after.to_string()),
                ("limit", "1"),
            ]);
        if let Some(through) = through {
            request = request.query(&[("through", through.to_string())]);
        }
        let page: sync_transport::Page =
            json(request.send().await?, sync_transport::MAX_PAGE_BYTES).await?;
        let high = cursor(&page.through)?;
        if page.server_identity != self.authorization.server
            || high < after
            || through.is_some_and(|value| value != high)
        {
            return Err(invalid());
        }
        if page.items.is_empty() {
            if high != after || page.next_after != after.to_string() || page.has_more {
                return Err(invalid());
            }
            self.authorization.position(&state.db).await?;

            return Ok(NextDelivery::Idle(page.through));
        }
        if page.items.len() != 1 {
            return Err(invalid());
        }
        let row = page.items.into_iter().next().ok_or_else(invalid)?;
        let sequence = cursor(&row.sequence)?;
        if sequence != after.checked_add(1).ok_or_else(invalid)?
            || sequence > high
            || page.next_after != row.sequence
            || page.has_more != (sequence < high)
        {
            return Err(invalid());
        }
        let delivery = Delivery {
            authorization: self.authorization.clone(),
            stream,
            after,
            through: high,
            sequence,
            digest: row.digest,
            issuer: row.issuer,
        };
        Ok(NextDelivery::Item(Box::new(delivery), row.item))
    }
    #[allow(clippy::too_many_lines, clippy::significant_drop_tightening)] // The stage lease must move into owned finalization.
    async fn pull(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
    ) -> Result<PullOutcome, ServiceError> {
        let (delivery, item) = match self.next_delivery(state, user, identity).await? {
            NextDelivery::Idle(through) => return Ok(PullOutcome::Idle { through }),
            NextDelivery::Item(delivery, item) => (*delivery, item),
        };
        self.apply_captured(state, user, delivery, item).await
    }

    async fn acknowledge_captured(
        &self,
        state: &AppState,
        delivery: &Delivery,
        item: &serde_json::Value,
    ) -> Result<Option<PullOutcome>, ServiceError> {
        if item.get("kind").and_then(serde_json::Value::as_str) != Some("observed_group") {
            return Ok(None);
        }
        let db = state.db.clone();
        let delivery = delivery.clone();
        let item = item.clone();
        // A disconnect can lose the response, but cannot cancel a receipt
        // commit halfway through its atomic cursor update.
        let acknowledgment = tokio::spawn(async move {
            sync_transport::observations::acknowledge_delivery(&db, delivery, item).await
        })
        .await
        .map_err(|_| invalid())??;

        Ok(acknowledgment.map(outcome))
    }

    #[allow(clippy::significant_drop_tightening)] // The stage lease must move into owned finalization.
    async fn apply_captured(
        &self,
        state: &AppState,
        user: &User,
        delivery: Delivery,
        item: serde_json::Value,
    ) -> Result<PullOutcome, ServiceError> {
        if item.get("kind").and_then(serde_json::Value::as_str) != Some("observed_group") {
            let state = state.clone();
            let result = tokio::spawn(async move {
                sync_transport::apply_delivery(&state.db, delivery, item).await
            })
            .await
            .map_err(|_| invalid())??;

            return Ok(outcome(result));
        }
        if let Some(acknowledgment) = self.acknowledge_captured(state, &delivery, &item).await? {
            return Ok(acknowledgment);
        }
        let incoming = self.download_observation(state, &delivery, &item).await?;
        let state = state.clone();
        let user = user.clone();
        // Own finalization through commit even if its waiting HTTP client disappears.
        let result = tokio::spawn(async move {
            sync_observation_service::install_delivery(state, user, delivery, item, incoming).await
        })
        .await
        .map_err(|_| invalid())??;
        Ok(outcome(result))
    }
}

fn outcome(result: sync_transport::Published) -> PullOutcome {
    if result.status == "already_present" {
        PullOutcome::AlreadyApplied {
            sequence: result.sequence,
        }
    } else {
        PullOutcome::Applied {
            sequence: result.sequence,
        }
    }
}
