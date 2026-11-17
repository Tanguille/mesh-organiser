use crate::{DbError, db_context::DbContext, model::user::User, user_db::SyncIdentity};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PeerBinding {
    pub local: SyncIdentity,
    pub remote: SyncIdentity,
}

/// Stores a caller-provided pin; it does not authenticate the remote identity.
pub async fn bind_peer(
    db: &DbContext,
    user: &User,
    expected_local: &SyncIdentity,
    remote: &SyncIdentity,
) -> Result<PeerBinding, DbError> {
    validate_identity(expected_local)?;
    validate_identity(remote)?;
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    validate_local(&mut transaction, user, expected_local).await?;
    sqlx::query("INSERT INTO sync_peer_bindings (user_id, local_instance_id, local_account_id, remote_instance_id, remote_account_id) VALUES (?, ?, ?, ?, ?) ON CONFLICT (user_id, local_instance_id, local_account_id, remote_instance_id, remote_account_id) DO NOTHING")
        .bind(user.id).bind(&expected_local.instance_id).bind(&expected_local.account_id).bind(&remote.instance_id).bind(&remote.account_id).execute(&mut *transaction).await?;
    transaction.commit().await?;

    Ok(PeerBinding {
        local: expected_local.clone(),
        remote: remote.clone(),
    })
}

pub async fn list_peers(
    db: &DbContext,
    user: &User,
    expected_local: &SyncIdentity,
) -> Result<Vec<PeerBinding>, DbError> {
    validate_identity(expected_local)?;
    let mut transaction = db.begin().await?;
    validate_local(&mut transaction, user, expected_local).await?;
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT remote_instance_id, remote_account_id FROM sync_peer_bindings WHERE user_id = ? AND local_instance_id = ? AND local_account_id = ? ORDER BY remote_instance_id, remote_account_id")
        .bind(user.id).bind(&expected_local.instance_id).bind(&expected_local.account_id).fetch_all(&mut *transaction).await?;
    let mut peers = Vec::with_capacity(rows.len());
    for (instance_id, account_id) in rows {
        let remote = SyncIdentity {
            instance_id,
            account_id,
        };
        validate_identity(&remote)?;
        peers.push(PeerBinding {
            local: expected_local.clone(),
            remote,
        });
    }
    transaction.commit().await?;

    Ok(peers)
}

pub(crate) fn validate_identity(identity: &SyncIdentity) -> Result<(), DbError> {
    fn valid(value: &str) -> bool {
        value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }
    if !valid(&identity.instance_id) || !valid(&identity.account_id) {
        return Err(DbError::Protocol("Invalid sync peer identity".into()));
    }

    Ok(())
}

pub(crate) async fn validate_local(
    db: &mut sqlx::SqliteConnection,
    user: &User,
    expected: &SyncIdentity,
) -> Result<(), DbError> {
    let identity: Option<SyncIdentity> = sqlx::query_as("SELECT i.instance_id, a.account_id FROM sync_instance i JOIN sync_accounts a ON a.user_id = ? WHERE i.singleton = 1")
        .bind(user.id).fetch_optional(db).await?;
    if identity.as_ref() != Some(expected) {
        return Err(DbError::Protocol("Invalid sync peer identity".into()));
    }

    Ok(())
}
