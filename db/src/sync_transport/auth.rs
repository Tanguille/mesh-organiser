use sqlx::SqliteConnection;

use crate::{
    DbError,
    db_context::DbContext,
    model::user::User,
    sync_peer,
    user_db::SyncIdentity,
    util::{is_lower_hex, sha256_hex},
};

use super::{WriterRegistration, failure};

pub(super) async fn account(
    db: &mut SqliteConnection,
    user: &User,
    expected: &str,
) -> Result<SyncIdentity, DbError> {
    let value:Option<SyncIdentity>=sqlx::query_as("SELECT i.instance_id,a.account_id FROM sync_instance i JOIN sync_accounts a ON a.user_id=? WHERE i.singleton=1 AND a.account_id=?")
        .bind(user.id).bind(expected).fetch_optional(db).await?;

    value.ok_or_else(|| failure("unauthorized"))
}

pub async fn register_writer(
    db: &DbContext,
    user: &User,
    expected_account: &str,
    client: &SyncIdentity,
) -> Result<WriterRegistration, DbError> {
    sync_peer::validate_identity(client)?;
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    let server_identity = account(&mut transaction, user, expected_account).await?;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_transport_writers WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&mut *transaction)
            .await?;
    if count >= 4096 {
        return Err(failure("quota_paused"));
    }
    let writer_id = crate::random_hex_32();
    let occupied:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM local_causal_writers WHERE replica_id=?) OR EXISTS(SELECT 1 FROM causal_events WHERE writer_id=?) OR EXISTS(SELECT 1 FROM causal_replica_owners WHERE replica_id=?)").bind(&writer_id).bind(&writer_id).bind(&writer_id).fetch_one(&mut *transaction).await?;
    if occupied {
        return Err(failure("author_blocked"));
    }
    let registration = WriterRegistration {
        server_identity,
        connection_id: crate::random_hex_32(),
        writer_id,
        writer_secret: format!("{}{}", crate::random_hex_32(), crate::random_hex_32()),
    };
    sqlx::query("INSERT INTO sync_transport_writers(writer_id,user_id,account_id,connection_id,secret_hash,client_instance_id,client_account_id) VALUES(?,?,?,?,?,?,?)")
        .bind(&registration.writer_id).bind(user.id).bind(expected_account).bind(&registration.connection_id).bind(sha256_hex(registration.writer_secret.as_bytes())).bind(&client.instance_id).bind(&client.account_id).execute(&mut *transaction).await?;
    transaction.commit().await?;

    Ok(registration)
}

#[derive(sqlx::FromRow)]
#[allow(clippy::struct_field_names)]
pub(super) struct Writer {
    pub writer_id: String,
    pub source_instance_id: String,
    pub source_account_id: String,
}
// The issuer assigns provenance; caller-supplied client ids are never authorship.
impl Writer {
    pub fn source(&self) -> SyncIdentity {
        SyncIdentity {
            instance_id: self.source_instance_id.clone(),
            account_id: self.source_account_id.clone(),
        }
    }
}
pub(super) async fn authorize(
    db: &mut SqliteConnection,
    user: &User,
    expected: &str,
    connection: &str,
    secret: &str,
) -> Result<(SyncIdentity, Writer), DbError> {
    let identity = account(db, user, expected).await?;
    if secret.len() != 64 {
        return Err(failure("forbidden"));
    }
    let writer:Option<Writer>=sqlx::query_as("SELECT writer_id,connection_id AS source_instance_id,account_id AS source_account_id FROM sync_transport_writers WHERE user_id=? AND account_id=? AND connection_id=? AND secret_hash=?")
        .bind(user.id).bind(expected).bind(connection).bind(sha256_hex(secret.as_bytes())).fetch_optional(db).await?;

    Ok((identity, writer.ok_or_else(|| failure("forbidden"))?))
}

pub async fn activate_writer(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    registration: &WriterRegistration,
) -> Result<(), DbError> {
    sync_peer::validate_identity(identity)?;
    sync_peer::validate_identity(&registration.server_identity)?;
    if !is_lower_hex(&registration.writer_secret, 64)
        || !is_lower_hex(&registration.connection_id, 32)
        || !is_lower_hex(&registration.writer_id, 32)
    {
        return Err(failure("invalid_registration"));
    }
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    sync_peer::validate_local(&mut transaction, user, identity).await?;
    let pinned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_peer_bindings WHERE user_id=? AND local_instance_id=? AND local_account_id=? AND remote_instance_id=? AND remote_account_id=?)")
        .bind(user.id).bind(&identity.instance_id).bind(&identity.account_id).bind(&registration.server_identity.instance_id).bind(&registration.server_identity.account_id).fetch_one(&mut *transaction).await?;
    if !pinned {
        return Err(failure("unverified_server"));
    }

    let offline: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sync_transport_local_authorities WHERE user_id=?)",
    )
    .bind(user.id)
    .fetch_one(&mut *transaction)
    .await?;
    if offline {
        return Err(failure("authority_conflict"));
    }
    // Never reactivate a past writer after restart: restored/cloned counters are unsafe.
    let occupied:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM local_causal_writers WHERE replica_id=?) OR EXISTS(SELECT 1 FROM causal_events WHERE writer_id=?) OR EXISTS(SELECT 1 FROM causal_replica_owners WHERE replica_id=?)")
        .bind(&registration.writer_id).bind(&registration.writer_id).bind(&registration.writer_id).fetch_one(&mut *transaction).await?;
    if occupied {
        return Err(failure("writer_already_used"));
    }
    sqlx::query("INSERT INTO sync_transport_local_writers(writer_id,user_id,local_instance_id,local_account_id,server_instance_id,server_account_id,connection_id,secret_hash) VALUES(?,?,?,?,?,?,?,?)")
        .bind(&registration.writer_id).bind(user.id).bind(&identity.instance_id).bind(&identity.account_id).bind(&registration.server_identity.instance_id).bind(&registration.server_identity.account_id).bind(&registration.connection_id).bind(sha256_hex(registration.writer_secret.as_bytes())).execute(&mut *transaction).await?;
    // Compatibility activation is account-scoped and expires on reopen.
    sqlx::query("INSERT INTO causal_account_writers(user_id,writer_id,counter) VALUES(?,?,0) ON CONFLICT(user_id) DO UPDATE SET writer_id=excluded.writer_id,counter=0")
        .bind(user.id).bind(&registration.writer_id).execute(&mut *transaction).await?;
    sqlx::query("INSERT INTO local_causal_writers(replica_id) VALUES(?)")
        .bind(&registration.writer_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;

    Ok(())
}
