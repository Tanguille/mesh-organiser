//! Finite original-item publication. Durable background scheduling remains gated.
use super::{AuthenticatedHub, PublicationOutcome, cursor, invalid, json};
use crate::{AppState, ServiceError, sync_observation_service::PreparedObservedGroup};
use db::{
    causal_envelope::{Dependency, Dot},
    causal_genesis::GenesisBundle,
    model::user::User,
    sync_transport::{self, publication::Publication},
    user_db::SyncIdentity,
};
use reqwest::{
    StatusCode,
    multipart::{Form, Part},
};
use tokio::io::AsyncReadExt;
use tokio_util::io::ReaderStream;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Acknowledgment {
    status: String,
    sequence: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Missing {
    status: String,
    missing: Vec<Dependency>,
}

impl AuthenticatedHub {
    pub async fn publish_observation(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        prepared: &PreparedObservedGroup,
    ) -> Result<PublicationOutcome, ServiceError> {
        let proof = self
            .publication(
                state,
                user,
                identity,
                prepared.group_event(),
                prepared.item(),
            )
            .await?;
        let body = self.publication_body(prepared.item(), &proof)?;
        let mut form = Form::new().text("request", body);
        // The caller's prepared snapshot owns the lease and staging capacity
        // throughout streaming/retry. Never reopen the mutable source original.
        for blob in prepared.blobs() {
            let file = tokio::fs::File::open(blob.path()).await?;
            if file.metadata().await?.len() != blob.size() {
                return Err(invalid());
            }
            let body = reqwest::Body::wrap_stream(ReaderStream::new(file.take(blob.size())));
            let part = Part::stream_with_length(body, blob.size());
            form = form.part(format!("blob.{}", blob.key()), part);
        }
        let response = self
            .client
            .post(format!(
                "{}/api/v2/sync/observations",
                self.authorization.origin
            ))
            .header("x-sync-writer-secret", self.authorization.secret())
            .multipart(form)
            .send()
            .await?;

        self.publication_response(state, response, &proof, prepared.item())
            .await
    }

    /// Empty observed batches still use the observation protocol and exact proof.
    pub(super) async fn publish_empty_observation(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        event: &Dot,
        item: &serde_json::Value,
    ) -> Result<PublicationOutcome, ServiceError> {
        if !sync_transport::observations::parse(item)?
            .contents()
            .is_empty()
        {
            return Err(invalid());
        }
        let proof = self.publication(state, user, identity, event, item).await?;
        let form = Form::new().text("request", self.publication_body(item, &proof)?);
        let response = self
            .client
            .post(format!(
                "{}/api/v2/sync/observations",
                self.authorization.origin
            ))
            .header("x-sync-writer-secret", self.authorization.secret())
            .multipart(form)
            .send()
            .await?;
        self.publication_response(state, response, &proof, item)
            .await
    }

    pub async fn publish_operation(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        event: &Dot,
    ) -> Result<PublicationOutcome, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }
        let bundle =
            db::causal_group_lifecycle::export_operation(&state.db, user, identity, event).await?;
        let item = sync_transport::encode_operation(&bundle)?;

        self.publish_item(state, user, identity, event, &item).await
    }

    pub async fn publish_genesis(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        bundle: &GenesisBundle,
    ) -> Result<PublicationOutcome, ServiceError> {
        let item = sync_transport::encode_genesis(bundle)?;

        self.publish_item(state, user, identity, bundle.event(), &item)
            .await
    }

    async fn publication(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        event: &Dot,
        item: &serde_json::Value,
    ) -> Result<Publication, ServiceError> {
        if !self.authorization.matches(user, identity) {
            return Err(invalid());
        }

        Ok(Publication::prepare(&state.db, &self.authorization, event, item).await?)
    }

    fn publication_body(
        &self,
        item: &serde_json::Value,
        proof: &Publication,
    ) -> Result<String, ServiceError> {
        let body = serde_json::to_string(&serde_json::json!({"protocol":1,"connection_id":self.authorization.connection,"writer_nonce":proof.nonce,"item":item}))
            .map_err(|_| invalid())?;
        if body.len() > sync_transport::MAX_WIRE_ITEM_BYTES {
            return Err(invalid());
        }

        Ok(body)
    }

    pub(super) async fn publish_item(
        &self,
        state: &AppState,
        user: &User,
        identity: &SyncIdentity,
        event: &Dot,
        item: &serde_json::Value,
    ) -> Result<PublicationOutcome, ServiceError> {
        let proof = self.publication(state, user, identity, event, item).await?;
        let response = self
            .client
            .post(format!("{}/api/v2/sync/items", self.authorization.origin))
            .header("x-sync-writer-secret", self.authorization.secret())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(self.publication_body(item, &proof)?)
            .send()
            .await?;

        self.publication_response(state, response, &proof, item)
            .await
    }

    async fn publication_response(
        &self,
        state: &AppState,
        mut response: reqwest::Response,
        proof: &Publication,
        item: &serde_json::Value,
    ) -> Result<PublicationOutcome, ServiceError> {
        if response.status() == StatusCode::CONFLICT {
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if bytes.len().saturating_add(chunk.len()) > sync_transport::MAX_WIRE_ITEM_BYTES {
                    return Err(invalid());
                }
                bytes.extend_from_slice(&chunk);
            }
            let missing: Missing = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if missing.status != "needs_dependencies"
                || missing.missing.is_empty()
                || missing.missing.len() > db::causal_envelope::MAX_CONTEXT_ITEMS
            {
                return Err(invalid());
            }
            sync_transport::publication::validate_link(&state.db, &self.authorization).await?;
            return Ok(PublicationOutcome::NeedsDependencies {
                missing: missing.missing,
            });
        }
        let acknowledgment: Acknowledgment = json(response, 4096).await?;
        if !matches!(
            acknowledgment.status.as_str(),
            "applied" | "already_present"
        ) {
            return Err(invalid());
        }
        let sequence = cursor(&acknowledgment.sequence)?;
        if sequence == 0 {
            return Err(invalid());
        }
        let page: sync_transport::Page = json(
            self.client
                .get(format!("{}/api/v2/sync/items", self.authorization.origin))
                .header("x-sync-writer-secret", self.authorization.secret())
                .query(&[
                    ("protocol", "1"),
                    ("connection_id", self.authorization.connection.as_str()),
                    ("after", &(sequence - 1).to_string()),
                    ("through", &acknowledgment.sequence),
                    ("limit", "1"),
                ])
                .send()
                .await?,
            sync_transport::MAX_PAGE_BYTES,
        )
        .await?;
        if page.server_identity != self.authorization.server
            || page.through != acknowledgment.sequence
            || page.next_after != acknowledgment.sequence
            || page.has_more
            || page.items.len() != 1
        {
            return Err(invalid());
        }
        let row = &page.items[0];
        if row.sequence != acknowledgment.sequence
            || row.digest != proof.digest
            || row.item != *item
            || row.issuer.connection_id != proof.issuer.connection_id
            || row.issuer.authority_writer_id != proof.issuer.authority_writer_id
            || row.issuer.writer_id != proof.issuer.writer_id
        {
            return Err(invalid());
        }
        // A remote commit may outlive cancellation/relink. Never report its ACK
        // for a different local account or advance an incoming stream here.
        sync_transport::publication::validate_link(&state.db, &self.authorization).await?;

        Ok(if acknowledgment.status == "already_present" {
            PublicationOutcome::AlreadyPresent {
                sequence: acknowledgment.sequence,
            }
        } else {
            PublicationOutcome::Published {
                sequence: acknowledgment.sequence,
            }
        })
    }
}
