//! Atomic local pairing state. Only the service HTTPS exchange authenticates a hub.
use super::{WriterRegistration, failure, offline, pull::Authorization};
use crate::{
    DbError,
    db_context::DbContext,
    model::user::{User, UserPermissions},
    sync_peer,
    user_db::SyncIdentity,
};

#[derive(PartialEq, Eq, sqlx::FromRow)]
struct Link {
    url: Option<String>,
    token: Option<String>,
    generation: i64,
    online: i64,
}

pub struct Pairing {
    user: User,
    identity: SyncIdentity,
    link: Link,
    registration: Option<WriterRegistration>,
}

async fn link(db: &mut sqlx::SqliteConnection, user: i64) -> Result<Link, DbError> {
    sqlx::query_as("SELECT u.user_sync_url AS url,u.user_sync_token AS token,g.generation,(u.user_permissions & ?) AS online FROM users u JOIN sync_link_generations g ON g.user_id=u.user_id WHERE u.user_id=?")
        .bind(i64::from(UserPermissions::OnlineAccount.bits())).bind(user).fetch_one(db).await
}

impl Pairing {
    pub async fn capture(
        db: &DbContext,
        user: &User,
        identity: &SyncIdentity,
    ) -> Result<Self, DbError> {
        let mut tx = db.begin().await?;
        sync_peer::validate_local(&mut tx, user, identity).await?;
        let link = link(&mut tx, user.id).await?;
        let row: Option<(String,String,String,String,String,String,String)> = sqlx::query_as("SELECT local_instance_id,local_account_id,server_instance_id,server_account_id,connection_id,authority_writer_id,secret FROM sync_transport_local_authorities WHERE user_id=?")
            .bind(user.id).fetch_optional(&mut *tx).await?;
        let registration = if let Some(row) = row {
            if row.0 != identity.instance_id || row.1 != identity.account_id {
                return Err(failure("stale_link"));
            }
            Some(WriterRegistration {
                server_identity: SyncIdentity {
                    instance_id: row.2,
                    account_id: row.3,
                },
                connection_id: row.4,
                writer_id: row.5,
                writer_secret: row.6,
            })
        } else {
            None
        };
        tx.commit().await?;

        Ok(Self {
            user: user.clone(),
            identity: identity.clone(),
            link,
            registration,
        })
    }

    #[must_use]
    pub const fn registration(&self) -> Option<&WriterRegistration> {
        self.registration.as_ref()
    }

    /// The caller must have authenticated this registration at the selected HTTPS origin.
    /// A snapshot or stored peer binding by itself is not remote authentication.
    pub async fn commit(
        self,
        db: &DbContext,
        origin: &str,
        token: &str,
        registration: &WriterRegistration,
    ) -> Result<Authorization, DbError> {
        let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
        sync_peer::validate_local(&mut tx, &self.user, &self.identity).await?;
        if link(&mut tx, self.user.id).await? != self.link {
            return Err(failure("stale_link"));
        }
        if let Some(previous) = &self.registration
            && (previous.server_identity != registration.server_identity
                || previous.connection_id != registration.connection_id
                || previous.writer_id != registration.writer_id
                || previous.writer_secret != registration.writer_secret)
        {
            return Err(failure("authority_conflict"));
        }
        // Updating a link revokes its active clock through the existing triggers.
        // Reinstalling the same authority preserves prior author delegation evidence.
        if self.link.url.as_deref() != Some(origin)
            || self.link.token.as_deref() != Some(token)
            || self.link.online != 0
        {
            let bits = i64::from(UserPermissions::OnlineAccount.bits() ^ u32::MAX);
            sqlx::query("UPDATE users SET user_sync_url=?,user_sync_token=?,user_last_sync=NULL,user_permissions=(user_permissions & ?) WHERE user_id=?")
                .bind(origin).bind(token).bind(bits).bind(self.user.id).execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO sync_peer_bindings(user_id,local_instance_id,local_account_id,remote_instance_id,remote_account_id) VALUES(?,?,?,?,?) ON CONFLICT DO NOTHING")
            .bind(self.user.id).bind(&self.identity.instance_id).bind(&self.identity.account_id).bind(&registration.server_identity.instance_id).bind(&registration.server_identity.account_id).execute(&mut *tx).await?;
        offline::install_authority_in(&mut tx, &self.user, &self.identity, registration).await?;
        let authorization =
            Authorization::capture_in(&mut tx, &self.user, &self.identity, origin, token).await?;
        tx.commit().await?;

        Ok(authorization)
    }
}
