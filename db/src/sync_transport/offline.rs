//! Account-scoped offline authority. A peer pin alone is not server authentication.
use sqlx::SqliteConnection;

use crate::{
    DbError, db_context::DbContext, model::user::User, sync_peer, user_db::SyncIdentity,
    util::is_lower_hex,
};

use super::{WriterRegistration, failure};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OfflineWriterProof {
    pub connection_id: String,
    pub writer_nonce: String,
}

pub struct StoredAuthority {
    registration: WriterRegistration,
}
impl std::fmt::Debug for StoredAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.registration.fmt(formatter)
    }
}
impl StoredAuthority {
    #[must_use]
    pub const fn server_identity(&self) -> &SyncIdentity {
        &self.registration.server_identity
    }
    #[must_use]
    pub fn connection_id(&self) -> &str {
        &self.registration.connection_id
    }
    #[must_use]
    pub fn writer_secret(&self) -> &str {
        &self.registration.writer_secret
    }
}
#[derive(sqlx::FromRow)]
struct LocalAuthority {
    user_id: i64,
    local_instance_id: String,
    local_account_id: String,
    server_instance_id: String,
    server_account_id: String,
    connection_id: String,
    authority_writer_id: String,
    secret: String,
}
impl LocalAuthority {
    fn registration(&self) -> WriterRegistration {
        WriterRegistration {
            server_identity: SyncIdentity {
                instance_id: self.server_instance_id.clone(),
                account_id: self.server_account_id.clone(),
            },
            connection_id: self.connection_id.clone(),
            writer_id: self.authority_writer_id.clone(),
            writer_secret: self.secret.clone(),
        }
    }
    fn same(&self, local: &SyncIdentity, registration: &WriterRegistration) -> bool {
        self.local_instance_id == local.instance_id
            && self.local_account_id == local.account_id
            && self.server_instance_id == registration.server_identity.instance_id
            && self.server_account_id == registration.server_identity.account_id
            && self.connection_id == registration.connection_id
            && self.authority_writer_id == registration.writer_id
            && self.secret == registration.writer_secret
    }
}

pub(super) fn derived_writer(
    server: &SyncIdentity,
    connection: &str,
    nonce: &str,
) -> Result<String, DbError> {
    if !is_lower_hex(nonce, 64) {
        return Err(failure("invalid_nonce"));
    }
    let digest = crate::util::sha256_hex(
        format!(
            "mesh-offline-writer-v1:{}:{}:{connection}:{nonce}",
            server.instance_id, server.account_id
        )
        .as_bytes(),
    );

    Ok(digest[..32].into())
}
async fn pin(
    db: &mut SqliteConnection,
    user: &User,
    local: &SyncIdentity,
    server: &SyncIdentity,
) -> Result<(), DbError> {
    sync_peer::validate_local(db, user, local).await?;
    let pinned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_peer_bindings WHERE user_id=? AND local_instance_id=? AND local_account_id=? AND remote_instance_id=? AND remote_account_id=?)")
        .bind(user.id).bind(&local.instance_id).bind(&local.account_id).bind(&server.instance_id).bind(&server.account_id).fetch_one(db).await?;
    if !pinned {
        return Err(failure("unverified_server"));
    }

    Ok(())
}
async fn authority(
    db: &mut SqliteConnection,
    user: i64,
) -> Result<Option<LocalAuthority>, DbError> {
    sqlx::query_as("SELECT user_id,local_instance_id,local_account_id,server_instance_id,server_account_id,connection_id,authority_writer_id,secret FROM sync_transport_local_authorities WHERE user_id=?").bind(user).fetch_optional(db).await
}
async fn rotate(db: &mut SqliteConnection, value: &LocalAuthority) -> Result<(), DbError> {
    let nonce = format!("{}{}", crate::random_hex_32(), crate::random_hex_32());
    let writer = derived_writer(
        &value.registration().server_identity,
        &value.connection_id,
        &nonce,
    )?;
    let occupied:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM local_causal_writers WHERE replica_id=?) OR EXISTS(SELECT 1 FROM causal_events WHERE writer_id=?) OR EXISTS(SELECT 1 FROM causal_replica_owners WHERE replica_id=?)")
        .bind(&writer).bind(&writer).bind(&writer).fetch_one(&mut *db).await?;
    if occupied {
        return Err(failure("writer_already_used"));
    }
    sqlx::query("INSERT INTO local_writer_delegations(writer_id,user_id,account_id,connection_id,writer_nonce,server_instance_id,server_account_id,authority_writer_id) VALUES(?,?,?,?,?,?,?,?)")
        .bind(&writer).bind(value.user_id).bind(&value.local_account_id).bind(&value.connection_id).bind(nonce).bind(&value.server_instance_id).bind(&value.server_account_id).bind(&value.authority_writer_id).execute(&mut *db).await?;
    sqlx::query("INSERT INTO local_causal_writers(replica_id) VALUES(?)")
        .bind(&writer)
        .execute(&mut *db)
        .await?;
    sqlx::query("INSERT INTO causal_account_writers(user_id,writer_id,counter) VALUES(?,?,0) ON CONFLICT(user_id) DO UPDATE SET writer_id=excluded.writer_id,counter=0")
        .bind(value.user_id).bind(writer).execute(db).await?;

    Ok(())
}

pub async fn rotate_all(db: &mut SqliteConnection) -> Result<(), DbError> {
    // Legacy one-shot grants cannot safely resume a copied/restored counter.
    sqlx::query("DELETE FROM causal_account_writers WHERE user_id NOT IN (SELECT user_id FROM sync_transport_local_authorities)").execute(&mut *db).await?;

    let rows:Vec<LocalAuthority>=sqlx::query_as("SELECT l.user_id,l.local_instance_id,l.local_account_id,l.server_instance_id,l.server_account_id,l.connection_id,l.authority_writer_id,l.secret FROM sync_transport_local_authorities l JOIN sync_accounts a ON a.user_id=l.user_id AND a.account_id=l.local_account_id JOIN sync_instance i ON i.singleton=1 AND i.instance_id=l.local_instance_id").fetch_all(&mut *db).await?;
    for value in rows {
        rotate(db, &value).await?;
    }

    Ok(())
}

pub async fn install_authority(
    db: &DbContext,
    user: &User,
    local: &SyncIdentity,
    registration: &WriterRegistration,
) -> Result<(), DbError> {
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    install_authority_in(&mut transaction, user, local, registration).await?;
    transaction.commit().await?;

    Ok(())
}

pub(super) async fn install_authority_in(
    db: &mut SqliteConnection,
    user: &User,
    local: &SyncIdentity,
    registration: &WriterRegistration,
) -> Result<(), DbError> {
    sync_peer::validate_identity(local)?;
    sync_peer::validate_identity(&registration.server_identity)?;
    if !is_lower_hex(&registration.writer_id, 32)
        || !is_lower_hex(&registration.connection_id, 32)
        || !is_lower_hex(&registration.writer_secret, 64)
    {
        return Err(failure("invalid_registration"));
    }
    pin(db, user, local, &registration.server_identity).await?;
    if let Some(value) = authority(db, user.id).await? {
        if !value.same(local, registration) {
            return Err(failure("authority_conflict"));
        }
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM causal_account_writers WHERE user_id=?)",
        )
        .bind(user.id)
        .fetch_one(&mut *db)
        .await?;
        if !present {
            return Err(failure("invalid_clock"));
        }

        return Ok(());
    }
    sqlx::query("INSERT INTO sync_transport_local_authorities(user_id,local_instance_id,local_account_id,server_instance_id,server_account_id,connection_id,authority_writer_id,secret) VALUES(?,?,?,?,?,?,?,?)")
        .bind(user.id).bind(&local.instance_id).bind(&local.account_id).bind(&registration.server_identity.instance_id).bind(&registration.server_identity.account_id).bind(&registration.connection_id).bind(&registration.writer_id).bind(&registration.writer_secret).execute(&mut *db).await?;
    let value = authority(db, user.id)
        .await?
        .ok_or_else(|| failure("invalid_clock"))?;
    rotate(db, &value).await?;

    Ok(())
}
pub async fn load_authority(
    db: &DbContext,
    user: &User,
    local: &SyncIdentity,
    connection_id: &str,
) -> Result<StoredAuthority, DbError> {
    let mut transaction = db.begin().await?;
    let value = authority(&mut transaction, user.id)
        .await?
        .ok_or_else(|| failure("authority_missing"))?;
    let registration = value.registration();
    pin(&mut transaction, user, local, &registration.server_identity).await?;
    if !value.same(local, &registration) || connection_id != registration.connection_id {
        return Err(failure("authority_missing"));
    }
    transaction.commit().await?;

    Ok(StoredAuthority { registration })
}
pub async fn writer_proof(
    db: &DbContext,
    user: &User,
    local: &SyncIdentity,
    dot: &crate::causal_envelope::Dot,
) -> Result<OfflineWriterProof, DbError> {
    let mut transaction = db.begin().await?;
    sync_peer::validate_local(&mut transaction, user, local).await?;
    let proof:Option<(String,String)>=sqlx::query_as("SELECT d.connection_id,d.writer_nonce FROM local_writer_delegations d JOIN causal_events e ON e.writer_id=d.writer_id AND e.user_id=d.user_id JOIN sync_transport_local_authorities a ON a.user_id=d.user_id AND a.local_account_id=d.account_id AND a.connection_id=d.connection_id AND a.server_instance_id=d.server_instance_id AND a.server_account_id=d.server_account_id WHERE d.user_id=? AND d.account_id=? AND d.writer_id=? AND e.counter=? AND e.encoding='portable_v2'")
        .bind(user.id).bind(&local.account_id).bind(&dot.writer_id).bind(dot.counter.parse::<i64>().map_err(|_|failure("invalid_dot"))?).fetch_optional(&mut *transaction).await?;
    let (connection_id, writer_nonce) = proof.ok_or_else(|| failure("unverified_author"))?;
    transaction.commit().await?;

    Ok(OfflineWriterProof {
        connection_id,
        writer_nonce,
    })
}

/// Preserve the exact derivation only in the same transaction as accepted events.
pub(super) async fn verify_delegation(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    authority: &str,
    writer: &str,
    nonce: &str,
) -> Result<(), DbError> {
    let occupied:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_transport_writers WHERE writer_id=?) OR EXISTS(SELECT 1 FROM local_causal_writers WHERE replica_id=?)")
        .bind(writer).bind(writer).fetch_one(&mut *db).await?;
    if occupied {
        return Err(failure("forbidden"));
    }
    sqlx::query("INSERT INTO sync_transport_delegations(writer_id,user_id,account_id,authority_writer_id,writer_nonce) VALUES(?,?,?,?,?) ON CONFLICT(writer_id) DO NOTHING")
        .bind(writer).bind(user.id).bind(&identity.account_id).bind(authority).bind(nonce).execute(&mut *db).await?;
    let exact:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_transport_delegations WHERE writer_id=? AND user_id=? AND account_id=? AND authority_writer_id=? AND writer_nonce=?)")
        .bind(writer).bind(user.id).bind(&identity.account_id).bind(authority).bind(nonce).fetch_one(db).await?;
    if !exact {
        return Err(failure("forbidden"));
    }

    Ok(())
}
