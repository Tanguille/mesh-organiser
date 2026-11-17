//! Verify a locally archived original before the service sends it over authenticated HTTPS.
use super::{
    auth, exchange, failure, observations,
    pull::{Authorization, Issuer},
};
use crate::{DbError, causal_envelope::Dot, db_context::DbContext};

pub struct Publication {
    pub nonce: String,
    pub issuer: Issuer,
    pub digest: String,
}

impl Publication {
    pub async fn prepare(
        db: &DbContext,
        authorization: &Authorization,
        event: &Dot,
        value: &serde_json::Value,
    ) -> Result<Self, DbError> {
        let mut tx = db.begin().await?;
        let result = Self::prepare_in(&mut tx, authorization, event, value).await?;
        tx.commit().await?;

        Ok(result)
    }
    pub(super) async fn prepare_in(
        db: &mut sqlx::SqliteConnection,
        authorization: &Authorization,
        event: &Dot,
        value: &serde_json::Value,
    ) -> Result<Self, DbError> {
        authorization.validate(db).await?;
        let (nonce, issuer) = Self::proof_in(db, authorization, event).await?;
        let original =
            if value.get("kind").and_then(serde_json::Value::as_str) == Some("observed_group") {
                observations::verify_publication(db, authorization, &issuer, value).await?
            } else {
                exchange::verify_publication(db, authorization, &issuer, value).await?
            };
        if !original {
            return Err(failure("unverified_author"));
        }
        let wire = serde_json::to_string(value).map_err(|_| failure("unsupported"))?;

        Ok(Self {
            nonce,
            issuer,
            digest: auth::digest(&wire),
        })
    }
    pub(super) async fn proof_in(
        db: &mut sqlx::SqliteConnection,
        authorization: &Authorization,
        event: &Dot,
    ) -> Result<(String, Issuer), DbError> {
        let proof: Option<(String, String)> = sqlx::query_as("SELECT d.writer_nonce,d.authority_writer_id FROM local_writer_delegations d JOIN sync_transport_local_authorities a ON a.user_id=d.user_id AND a.local_account_id=d.account_id AND a.connection_id=d.connection_id AND a.authority_writer_id=d.authority_writer_id AND a.server_instance_id=d.server_instance_id AND a.server_account_id=d.server_account_id WHERE d.user_id=? AND d.account_id=? AND d.writer_id=? AND a.connection_id=?")
            .bind(authorization.user.id).bind(&authorization.identity.account_id).bind(&event.writer_id).bind(&authorization.connection).fetch_optional(db).await?;
        let (nonce, authority_writer_id) = proof.ok_or_else(|| failure("unverified_author"))?;
        let issuer = Issuer {
            connection_id: authorization.connection.clone(),
            authority_writer_id,
            writer_id: event.writer_id.clone(),
        };
        Ok((nonce, issuer))
    }
}

/// Recheck after the network wait without creating a pull stream or acknowledging history.
pub async fn validate_link(db: &DbContext, authorization: &Authorization) -> Result<(), DbError> {
    let mut tx = db.begin().await?;
    authorization.validate(&mut tx).await?;
    tx.commit().await?;

    Ok(())
}
