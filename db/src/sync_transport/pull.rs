//! Local authorization snapshots; only the service HTTPS client authenticates deliveries.
use super::{auth, failure};
use crate::{DbError, db_context::DbContext, model::user::User, user_db::SyncIdentity};
use sqlx::SqliteConnection;

#[derive(Clone)]
pub struct Authorization {
    pub(crate) user: User,
    pub(crate) identity: SyncIdentity,
    pub origin: String,
    pub server: SyncIdentity,
    pub connection: String,
    secret: String,
    token: String,
    generation: i64,
}
impl Authorization {
    /// Capture only an already installed authority for the actual local writer.
    /// This cannot enroll old events or grant a new writer permission retroactively.
    pub(crate) async fn recording_key(
        db: &mut SqliteConnection,
        user: &User,
        writer: &str,
    ) -> Result<Option<String>, DbError> {
        let row: Option<(String,String,String,String,String,String,i64)> = sqlx::query_as("SELECT a.local_instance_id,a.local_account_id,u.user_sync_url,a.server_instance_id,a.server_account_id,a.connection_id,g.generation FROM sync_transport_local_authorities a JOIN local_writer_delegations d ON d.user_id=a.user_id AND d.account_id=a.local_account_id AND d.connection_id=a.connection_id AND d.authority_writer_id=a.authority_writer_id AND d.server_instance_id=a.server_instance_id AND d.server_account_id=a.server_account_id JOIN sync_accounts s ON s.user_id=a.user_id AND s.account_id=a.local_account_id JOIN sync_instance i ON i.singleton=1 AND i.instance_id=a.local_instance_id JOIN users u ON u.user_id=a.user_id JOIN sync_link_generations g ON g.user_id=a.user_id WHERE a.user_id=? AND d.writer_id=? AND u.user_sync_url IS NOT NULL AND u.user_sync_token IS NOT NULL")
            .bind(user.id).bind(writer).fetch_optional(db).await?;

        Ok(row.map(
            |(
                instance_id,
                account_id,
                origin,
                server_instance,
                server_account,
                connection,
                generation,
            )| {
                identity_choice_key(
                    &SyncIdentity {
                        instance_id,
                        account_id,
                    },
                    &origin,
                    &SyncIdentity {
                        instance_id: server_instance,
                        account_id: server_account,
                    },
                    &connection,
                    generation,
                )
            },
        ))
    }

    pub(crate) fn identity_choice_key(&self) -> String {
        identity_choice_key(
            &self.identity,
            &self.origin,
            &self.server,
            &self.connection,
            self.generation,
        )
    }
    #[must_use]
    pub fn matches(&self, user: &User, identity: &SyncIdentity) -> bool {
        self.user.id == user.id && self.identity == *identity
    }
    #[must_use]
    pub fn secret(&self) -> &str {
        &self.secret
    }
    pub async fn capture(
        db: &DbContext,
        user: &User,
        identity: &SyncIdentity,
        origin: &str,
        token: &str,
    ) -> Result<Self, DbError> {
        let mut tx = db.begin().await?;
        let result = Self::capture_in(&mut tx, user, identity, origin, token).await?;
        tx.commit().await?;

        Ok(result)
    }
    pub(super) async fn capture_in(
        db: &mut SqliteConnection,
        user: &User,
        identity: &SyncIdentity,
        origin: &str,
        token: &str,
    ) -> Result<Self, DbError> {
        crate::sync_peer::validate_local(db, user, identity).await?;
        let row:(String,String,String,String,i64)=sqlx::query_as("SELECT a.server_instance_id,a.server_account_id,a.connection_id,a.secret,g.generation FROM sync_transport_local_authorities a JOIN users u ON u.user_id=a.user_id JOIN sync_link_generations g ON g.user_id=a.user_id WHERE a.user_id=? AND a.local_instance_id=? AND a.local_account_id=? AND u.user_sync_url=? AND u.user_sync_token=?")
            .bind(user.id).bind(&identity.instance_id).bind(&identity.account_id).bind(origin).bind(token).fetch_one(db).await?;
        Ok(Self {
            user: user.clone(),
            identity: identity.clone(),
            origin: origin.into(),
            server: SyncIdentity {
                instance_id: row.0,
                account_id: row.1,
            },
            connection: row.2,
            secret: row.3,
            token: token.into(),
            generation: row.4,
        })
    }
    pub(crate) async fn validate(&self, db: &mut SqliteConnection) -> Result<(), DbError> {
        crate::sync_peer::validate_local(db, &self.user, &self.identity).await?;
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_transport_local_authorities a JOIN users u ON u.user_id=a.user_id JOIN sync_link_generations g ON g.user_id=a.user_id WHERE a.user_id=? AND a.local_instance_id=? AND a.local_account_id=? AND a.server_instance_id=? AND a.server_account_id=? AND a.connection_id=? AND a.secret=? AND g.generation=? AND u.user_sync_url=? AND u.user_sync_token=?)")
          .bind(self.user.id).bind(&self.identity.instance_id).bind(&self.identity.account_id).bind(&self.server.instance_id).bind(&self.server.account_id).bind(&self.connection).bind(&self.secret).bind(self.generation).bind(&self.origin).bind(&self.token).fetch_one(db).await?;
        if !valid {
            return Err(failure("stale_link"));
        }
        Ok(())
    }
    pub async fn position(&self, db: &DbContext) -> Result<(String, i64, Option<i64>), DbError> {
        let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
        self.validate(&mut tx).await?;
        sqlx::query("INSERT INTO sync_pull_streams(stream_id,user_id,origin,server_instance_id,server_account_id) VALUES(?,?,?,?,?) ON CONFLICT(user_id,origin,server_instance_id,server_account_id) DO NOTHING").bind(crate::random_hex_32()).bind(self.user.id).bind(&self.origin).bind(&self.server.instance_id).bind(&self.server.account_id).execute(&mut *tx).await?;
        let result=sqlx::query_as("SELECT stream_id,last_applied,through FROM sync_pull_streams WHERE user_id=? AND origin=? AND server_instance_id=? AND server_account_id=?").bind(self.user.id).bind(&self.origin).bind(&self.server.instance_id).bind(&self.server.account_id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub(crate) fn issuer_origin(&self, issuer: &Issuer) -> SyncIdentity {
        let encoded = serde_json::to_string(&(
            &self.origin,
            &self.server.instance_id,
            &self.server.account_id,
            &issuer.connection_id,
        ))
        .expect("string tuple serializes");
        SyncIdentity {
            instance_id: auth::digest(&format!("mesh-pull-issuer-v1:{encoded}"))[..32].into(),
            account_id: self.server.account_id.clone(),
        }
    }
}
fn identity_choice_key(
    identity: &SyncIdentity,
    origin: &str,
    server: &SyncIdentity,
    connection: &str,
    generation: i64,
) -> String {
    serde_json::to_string(&(identity, origin, server, connection, generation))
        .expect("identity tuple serializes")
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Issuer {
    pub connection_id: String,
    pub authority_writer_id: String,
    pub writer_id: String,
}
#[derive(Clone)]
pub struct Delivery {
    pub authorization: Authorization,
    pub stream: String,
    pub after: i64,
    pub through: i64,
    pub sequence: i64,
    pub digest: String,
    pub issuer: Issuer,
}
impl Delivery {
    pub(crate) async fn validate(
        &self,
        db: &mut SqliteConnection,
        item_kind: &str,
        item_id: &str,
    ) -> Result<bool, DbError> {
        self.authorization.validate(db).await?;
        if self.sequence
            != self
                .after
                .checked_add(1)
                .ok_or_else(|| failure("invalid_cursor"))?
            || self.through < self.sequence
            || !auth::canonical_hex(&self.digest, 64)
            || [
                &self.issuer.connection_id,
                &self.issuer.authority_writer_id,
                &self.issuer.writer_id,
            ]
            .iter()
            .any(|v| !auth::canonical_hex(v, 32))
        {
            return Err(failure("invalid_cursor"));
        }
        // A weaker first-proof path must never override authenticated issuer
        // evidence already retained for this immutable item, even on another hub.
        let contradictory:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_pull_receipts r JOIN sync_pull_streams s USING(stream_id) WHERE s.user_id=? AND r.kind=? AND r.item_id=? AND (r.digest<>? OR r.issuer_connection_id<>? OR r.authority_writer_id<>? OR r.writer_id<>?))")
            .bind(self.authorization.user.id).bind(item_kind).bind(item_id).bind(&self.digest).bind(&self.issuer.connection_id).bind(&self.issuer.authority_writer_id).bind(&self.issuer.writer_id).fetch_one(&mut *db).await?;
        if contradictory {
            return Err(failure("identity_blocked"));
        }
        let prior:Option<(String,String,String,String,String,String)>=sqlx::query_as("SELECT kind,item_id,digest,issuer_connection_id,authority_writer_id,writer_id FROM sync_pull_receipts WHERE stream_id=? AND sequence=?")
            .bind(&self.stream).bind(self.sequence).fetch_optional(&mut *db).await?;
        if let Some((kind, id, digest, connection, authority, writer)) = prior {
            let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_pull_streams WHERE stream_id=? AND user_id=? AND origin=? AND server_instance_id=? AND server_account_id=? AND last_applied>=?)")
                .bind(&self.stream).bind(self.authorization.user.id).bind(&self.authorization.origin).bind(&self.authorization.server.instance_id).bind(&self.authorization.server.account_id).bind(self.sequence).fetch_one(&mut *db).await?;
            if owned
                && kind == item_kind
                && id == item_id
                && digest == self.digest
                && connection == self.issuer.connection_id
                && authority == self.issuer.authority_writer_id
                && writer == self.issuer.writer_id
            {
                return Ok(true);
            }
            return Err(failure("identity_blocked"));
        }
        let position:Option<(i64,Option<i64>)>=sqlx::query_as("SELECT last_applied,through FROM sync_pull_streams WHERE stream_id=? AND user_id=? AND origin=? AND server_instance_id=? AND server_account_id=?").bind(&self.stream).bind(self.authorization.user.id).bind(&self.authorization.origin).bind(&self.authorization.server.instance_id).bind(&self.authorization.server.account_id).fetch_optional(db).await?;
        if position.is_none_or(|(after, through)| {
            after != self.after || through.is_some_and(|v| v != self.through)
        }) {
            return Err(failure("stale_cursor"));
        }
        Ok(false)
    }
    /// A URL change does not erase a previously authenticated receipt. This is
    /// historical delivery evidence, never permission to merge a current row by id.
    pub(crate) async fn previously_applied(
        &self,
        db: &mut SqliteConnection,
        kind: &str,
        id: &str,
    ) -> Result<bool, DbError> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_pull_receipts r JOIN sync_pull_streams s USING(stream_id) WHERE r.kind=? AND r.item_id=? AND r.digest=? AND r.issuer_connection_id=? AND r.authority_writer_id=? AND r.writer_id=? AND s.user_id=? AND s.server_instance_id=? AND s.server_account_id=? AND s.last_applied>=r.sequence)")
            .bind(kind).bind(id).bind(&self.digest).bind(&self.issuer.connection_id).bind(&self.issuer.authority_writer_id).bind(&self.issuer.writer_id).bind(self.authorization.user.id).bind(&self.authorization.server.instance_id).bind(&self.authorization.server.account_id).fetch_one(db).await
    }
    pub(crate) async fn commit(
        &self,
        db: &mut SqliteConnection,
        kind: &str,
        id: &str,
    ) -> Result<(), DbError> {
        sqlx::query("INSERT INTO sync_pull_receipts(stream_id,sequence,kind,item_id,digest,issuer_connection_id,authority_writer_id,writer_id) VALUES(?,?,?,?,?,?,?,?)").bind(&self.stream).bind(self.sequence).bind(kind).bind(id).bind(&self.digest).bind(&self.issuer.connection_id).bind(&self.issuer.authority_writer_id).bind(&self.issuer.writer_id).execute(&mut *db).await?;
        sqlx::query("UPDATE sync_pull_streams SET last_applied=?,through=? WHERE stream_id=?")
            .bind(self.sequence)
            .bind((self.sequence < self.through).then_some(self.through))
            .bind(&self.stream)
            .execute(db)
            .await?;
        Ok(())
    }
}
