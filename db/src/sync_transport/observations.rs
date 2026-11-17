//! Authenticated metadata transactions. The service verifies physical content.
use sqlx::SqliteConnection;

use crate::{
    DbError,
    causal_genesis::{
        identity,
        populated::{ModelBlobBinding, ObservedPopulatedGroupBundle, transport},
    },
    db_context::DbContext,
    model::{blob::Blob, user::User},
    user_db::SyncIdentity,
    util::is_lower_hex,
};

use super::{Published, auth, failure, offline};

pub use transport::BlobContent;

pub const MAX_BLOB_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

pub struct ObservationPlan {
    pub(crate) bundle: ObservedPopulatedGroupBundle,
    contents: Vec<BlobContent>,
    pub(crate) digest: String,
}
impl ObservationPlan {
    #[must_use]
    pub fn contents(&self) -> &[BlobContent] {
        &self.contents
    }
}
pub fn encode(
    bundle: &ObservedPopulatedGroupBundle,
    contents: Vec<BlobContent>,
) -> Result<serde_json::Value, DbError> {
    let value = transport::encode_bundle(bundle, contents)?;
    parse(&value)?;

    Ok(value)
}
pub fn parse(value: &serde_json::Value) -> Result<ObservationPlan, DbError> {
    let wire = serde_json::to_string(value).map_err(|_| failure("unsupported"))?;
    if wire.len() > super::MAX_WIRE_ITEM_BYTES {
        return Err(failure("too_large"));
    }
    let (bundle, contents) = transport::decode(value.clone())?;
    // Reconstruction must reproduce the exact JSON item whose digest was acknowledged.
    // In particular, omitted optional fields cannot silently become explicit nulls.
    if transport::encode_bundle(&bundle, contents.clone())? != *value {
        return Err(failure("unsupported"));
    }
    let descriptors = transport::descriptors(&bundle)?;
    let mut hashes = std::collections::BTreeSet::new();
    let mut total = 0_u64;
    let mut previous = None;
    for content in &contents {
        if !is_lower_hex(&content.sha256, 64)
            || content.hash != content.sha256[..32]
            || !matches!(
                content.filetype.as_str(),
                "stl"
                    | "obj"
                    | "gcode"
                    | "step"
                    | "stp"
                    | "3mf"
                    | "stl.zip"
                    | "obj.zip"
                    | "gcode.zip"
                    | "step.zip"
                    | "stp.zip"
            )
            || content.size < 0
            || !hashes.insert(content.hash.clone())
            || previous
                .as_ref()
                .is_some_and(|last: &String| last >= &content.hash)
            || !descriptors.iter().any(|(_, blob)| {
                blob.hash == content.hash
                    && blob.filetype == content.filetype
                    && blob.size == content.size
            })
        {
            return Err(failure("unsupported"));
        }
        let size = u64::try_from(content.size).map_err(|_| failure("unsupported"))?;
        total = total.saturating_add(size);
        if size > MAX_BLOB_BYTES || total > MAX_TOTAL_BYTES {
            return Err(failure("too_large"));
        }
        previous = Some(content.hash.clone());
    }
    if descriptors.iter().any(|(_, blob)| {
        !contents.iter().any(|content| {
            blob.hash == content.hash
                && blob.filetype == content.filetype
                && blob.size == content.size
        })
    }) {
        return Err(failure("unsupported"));
    }

    Ok(ObservationPlan {
        bundle,
        contents,
        digest: crate::util::sha256_hex(wire.as_bytes()),
    })
}
async fn prior(
    db: &mut SqliteConnection,
    user: &User,
    plan: &ObservationPlan,
) -> Result<Option<String>, DbError> {
    let row:Option<(i64,String)>=sqlx::query_as("SELECT sequence,digest FROM sync_transport_log WHERE user_id=? AND kind='observed_group' AND item_id=?").bind(user.id).bind(transport::batch_id(&plan.bundle)).fetch_optional(db).await?;
    match row {
        Some((sequence, digest)) if digest == plan.digest => Ok(Some(sequence.to_string())),
        Some(_) => Err(failure("identity_blocked")),
        None => Ok(None),
    }
}
pub(crate) fn author(plan: &ObservationPlan, writer: &str) -> Result<(), DbError> {
    if transport::roots(&plan.bundle)
        .iter()
        .any(|root| root.event.writer_id != writer)
    {
        return Err(failure("forbidden"));
    }

    Ok(())
}
pub async fn inspect(
    db: &DbContext,
    user: &User,
    account: &str,
    connection: &str,
    secret: &str,
    nonce: Option<&str>,
    value: serde_json::Value,
) -> Result<ObservationPlan, DbError> {
    let plan = parse(&value)?;
    let mut transaction = db.begin().await?;
    let (identity, writer) =
        auth::authorize(&mut transaction, user, account, connection, secret).await?;
    if prior(&mut transaction, user, &plan).await?.is_none() {
        let event_writer = match nonce {
            Some(nonce) => offline::derived_writer(&identity, connection, nonce)?,
            None => writer.writer_id,
        };
        author(&plan, &event_writer)?;
    }
    transaction.commit().await?;

    Ok(plan)
}
pub struct ObservationTransaction {
    tx: sqlx::Transaction<'static, sqlx::Sqlite>,
    user: User,
    identity: SyncIdentity,
    origin: SyncIdentity,
    authority: String,
    writer: String,
    plan: ObservationPlan,
    prior: Option<String>,
    delivery: Option<super::pull::Delivery>,
    historical_echo: bool,
}
pub async fn begin(
    db: &DbContext,
    user: &User,
    account: &str,
    connection: &str,
    secret: &str,
    nonce: Option<&str>,
    value: serde_json::Value,
) -> Result<ObservationTransaction, DbError> {
    let plan = parse(&value)?;
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    let (identity, writer) =
        auth::authorize(&mut transaction, user, account, connection, secret).await?;
    let event_writer = match nonce {
        Some(nonce) => offline::derived_writer(&identity, connection, nonce)?,
        None => writer.writer_id.clone(),
    };
    let prior = prior(&mut transaction, user, &plan).await?;
    if prior.is_none() {
        author(&plan, &event_writer)?;
        if let Some(nonce) = nonce {
            offline::verify_delegation(
                &mut transaction,
                user,
                &identity,
                &writer.writer_id,
                &event_writer,
                nonce,
            )
            .await?;
        }
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sync_transport_log WHERE user_id=?")
                .bind(user.id)
                .fetch_one(&mut *transaction)
                .await?;
        if count >= 4096 {
            return Err(failure("quota_paused"));
        }
    }

    Ok(ObservationTransaction {
        tx: transaction,
        user: user.clone(),
        identity,
        origin: writer.source(),
        authority: writer.writer_id,
        writer: event_writer,
        plan,
        prior,
        delivery: None,
        historical_echo: false,
    })
}
impl ObservationTransaction {
    #[must_use]
    pub const fn acknowledgment_only(&self) -> bool {
        self.prior.is_some() || self.historical_echo
    }

    #[must_use]
    pub fn contents(&self) -> &[BlobContent] {
        &self.plan.contents
    }
    pub async fn existing_blob(&mut self, hash: &str) -> Result<Option<Blob>, DbError> {
        sqlx::query_as("SELECT * FROM blobs WHERE blob_sha256=?")
            .bind(hash)
            .fetch_optional(&mut *self.tx)
            .await
    }
    pub async fn create_blob(
        &mut self,
        content: &BlobContent,
        path: &str,
        device: u64,
        inode: u64,
    ) -> Result<i64, DbError> {
        if self.prior.is_some() || !self.plan.contents.contains(content) {
            return Err(failure("identity_blocked"));
        }
        let id=sqlx::query("INSERT INTO blobs(blob_sha256,blob_filetype,blob_size,blob_added,blob_path) VALUES(?,?,?,?,?)")
            .bind(&content.hash).bind(&content.filetype).bind(content.size).bind(crate::time_now()).bind(path).execute(&mut *self.tx).await?.last_insert_rowid();
        sqlx::query(
            "INSERT INTO sync_owned_blob_files(path,blob_id,sha256,device,inode) VALUES(?,?,?,?,?)",
        )
        .bind(path)
        .bind(id)
        .bind(&content.sha256)
        .bind(device.to_string())
        .bind(inode.to_string())
        .execute(&mut *self.tx)
        .await?;

        Ok(id)
    }
    #[allow(clippy::too_many_lines)] // Domain projection and delivery acknowledgment share one transaction.
    pub async fn commit(mut self, blobs: Vec<(String, i64)>) -> Result<Published, DbError> {
        if let Some(sequence) = self.prior {
            self.tx.commit().await?;

            return Ok(Published {
                status: "already_present",
                sequence,
            });
        }
        if self.historical_echo {
            let delivery = self
                .delivery
                .as_ref()
                .ok_or_else(|| failure("identity_blocked"))?;
            delivery
                .commit(
                    &mut self.tx,
                    "observed_group",
                    transport::batch_id(&self.plan.bundle),
                )
                .await?;
            let sequence = delivery.sequence.to_string();
            self.tx.commit().await?;

            return Ok(Published {
                status: "already_present",
                sequence,
            });
        }
        let bindings = self.blob_bindings(&blobs)?;
        let retained = self.delivery.is_some()
            && transport::retain_installed_baseline(
                &mut self.tx,
                &self.user,
                &self.identity,
                &self.plan.bundle,
                &self.plan.contents,
                &bindings,
            )
            .await?;
        if !retained {
            transport::install(
                &mut self.tx,
                &self.user,
                &self.identity,
                &self.origin,
                &mut self.plan.bundle,
                &bindings,
            )
            .await?;
        }
        self.record_contents(&blobs).await?;
        let sequence = if let Some(delivery) = &self.delivery {
            delivery
                .commit(
                    &mut self.tx,
                    "observed_group",
                    transport::batch_id(&self.plan.bundle),
                )
                .await?;
            delivery.sequence
        } else {
            let sequence: i64 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(sequence),0)+1 FROM sync_transport_log WHERE user_id=?",
            )
            .bind(self.user.id)
            .fetch_one(&mut *self.tx)
            .await?;
            sqlx::query("INSERT INTO sync_transport_log(user_id,sequence,kind,item_id,digest,authority_writer_id,writer_id) VALUES(?,?,'observed_group',?,?,?,?)")
            .bind(self.user.id).bind(sequence).bind(transport::batch_id(&self.plan.bundle)).bind(self.plan.digest).bind(self.authority).bind(self.writer).execute(&mut *self.tx).await?;
            sequence
        };
        self.tx.commit().await?;

        Ok(Published {
            status: if retained {
                "already_present"
            } else {
                "applied"
            },
            sequence: sequence.to_string(),
        })
    }
    fn blob_bindings(&self, blobs: &[(String, i64)]) -> Result<Vec<ModelBlobBinding>, DbError> {
        if blobs.len() != self.plan.contents.len() {
            return Err(failure("identity_blocked"));
        }
        for content in &self.plan.contents {
            if blobs
                .iter()
                .filter(|(hash, _)| hash == &content.hash)
                .count()
                != 1
            {
                return Err(failure("identity_blocked"));
            }
        }
        transport::descriptors(&self.plan.bundle)?
            .into_iter()
            .map(|(entity, blob)| {
                let id = blobs
                    .iter()
                    .find(|(hash, _)| *hash == blob.hash)
                    .ok_or_else(|| failure("identity_blocked"))?
                    .1;
                Ok(ModelBlobBinding {
                    entity,
                    local_blob_id: id,
                })
            })
            .collect::<Result<Vec<_>, DbError>>()
    }

    /// The decision and exact transfer are checked again after network I/O; all
    /// blob bindings, domain changes and the pull receipt share this transaction.
    pub async fn commit_identity(
        mut self,
        decision_id: &str,
        decision: identity::Decision,
        blobs: Vec<(String, i64)>,
        local_source: Option<&identity::LocalObservationStage>,
    ) -> Result<identity::DecisionOutcome, DbError> {
        let delivery = self
            .delivery
            .as_ref()
            .ok_or_else(|| failure("identity_blocked"))?;
        let authorization = delivery.authorization.clone();
        // The persisted choice must name the same authenticated item whose bytes
        // were verified; matching child descriptors alone is not sufficient.
        identity::verify_transfer_in(&mut self.tx, &authorization, decision_id, delivery).await?;
        if let Some(result) =
            identity::retry_in(&mut self.tx, &authorization, decision_id, decision).await?
        {
            self.tx.commit().await?;

            return Ok(result);
        }
        if self.acknowledgment_only() {
            return Err(failure("identity_blocked"));
        }
        let bindings = self.blob_bindings(&blobs)?;
        let result = identity::finish_in(
            &mut self.tx,
            &authorization,
            decision_id,
            decision,
            &bindings,
            local_source,
        )
        .await?;
        self.record_contents(&blobs).await?;
        self.tx.commit().await?;

        Ok(result)
    }

    async fn record_contents(&mut self, blobs: &[(String, i64)]) -> Result<(), DbError> {
        for content in &self.plan.contents {
            let blob_id = blobs
                .iter()
                .find(|(hash, _)| hash == &content.hash)
                .ok_or_else(|| failure("identity_blocked"))?
                .1;
            let existing:Option<(String,i64,String,Option<i64>)>=sqlx::query_as("SELECT filetype,size,sha256,blob_id FROM sync_observation_contents WHERE user_id=? AND batch_id=? AND hash=?")
                .bind(self.user.id).bind(transport::batch_id(&self.plan.bundle)).bind(&content.hash).fetch_optional(&mut *self.tx).await?;
            if let Some((filetype, size, sha256, stored_blob)) = existing {
                if filetype != content.filetype
                    || size != content.size
                    || sha256 != content.sha256
                    || stored_blob != Some(blob_id)
                {
                    return Err(failure("identity_blocked"));
                }
                continue;
            }
            sqlx::query("INSERT INTO sync_observation_contents(user_id,batch_id,hash,filetype,size,sha256,blob_id) VALUES(?,?,?,?,?,?,?)")
                .bind(self.user.id).bind(transport::batch_id(&self.plan.bundle)).bind(&content.hash).bind(&content.filetype).bind(content.size).bind(&content.sha256).bind(blob_id).execute(&mut *self.tx).await?;
        }

        Ok(())
    }
}
pub async fn path_committed(db: &DbContext, path: &str) -> Result<bool, DbError> {
    // Serialize with any commit whose caller lost its result; a read snapshot alone
    // could observe the state before an in-flight writer finishes committing.
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    let committed = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sync_owned_blob_files WHERE path=? AND blob_id IS NOT NULL)",
    )
    .bind(path)
    .fetch_one(&mut *transaction)
    .await?;
    transaction.commit().await?;

    Ok(committed)
}
#[derive(sqlx::FromRow)]
pub struct OwnedBlobFile {
    pub path: String,
    pub device: String,
    pub inode: String,
}
pub async fn orphan_paths(db: &DbContext) -> Result<Vec<OwnedBlobFile>, DbError> {
    sqlx::query_as("SELECT path,device,inode FROM sync_owned_blob_files WHERE blob_id IS NULL")
        .fetch_all(db)
        .await
}

/// Owner-scoped source descriptors; observing/enrolling revalidates the captured state.
pub async fn source_blobs(db: &DbContext, user: &User, group: i64) -> Result<Vec<Blob>, DbError> {
    sqlx::query_as("SELECT DISTINCT b.* FROM blobs b JOIN models m ON m.model_blob_id=b.blob_id JOIN models_group g ON g.group_id=m.model_group_id WHERE m.model_user_id=? AND g.group_user_id=? AND g.group_id=? ORDER BY b.blob_sha256")
        .bind(user.id).bind(user.id).bind(group).fetch_all(db).await
}

/// A credential authorizes only this owner's committed observation contents.
#[allow(clippy::too_many_arguments)]
pub async fn download_blob(
    db: &DbContext,
    user: &User,
    account: &str,
    connection: &str,
    secret: &str,
    sequence: i64,
    key: &str,
) -> Result<(BlobContent, Blob), DbError> {
    if !is_lower_hex(key, 64) || sequence <= 0 {
        return Err(failure("unsupported"));
    }
    let mut transaction = db.begin().await?;
    auth::authorize(&mut transaction, user, account, connection, secret).await?;
    let row: Option<(String,String,i64,String,Option<i64>)> = sqlx::query_as("SELECT c.hash,c.filetype,c.size,c.sha256,c.blob_id FROM sync_observation_contents c JOIN sync_transport_log l ON l.user_id=c.user_id AND l.item_id=c.batch_id AND l.kind='observed_group' WHERE c.user_id=? AND l.sequence=? AND c.sha256=?")
        .bind(user.id).bind(sequence).bind(key).fetch_optional(&mut *transaction).await?;
    let (hash, filetype, size, sha256, id) = row.ok_or_else(|| failure("forbidden"))?;
    let id = id.ok_or_else(|| failure("identity_blocked"))?;
    let blob = sqlx::query_as("SELECT * FROM blobs WHERE blob_id=?")
        .bind(id)
        .fetch_one(&mut *transaction)
        .await?;
    transaction.commit().await?;

    Ok((
        BlobContent {
            hash,
            filetype,
            size,
            sha256,
        },
        blob,
    ))
}

pub async fn forget_orphan_path(db: &DbContext, path: &str) -> Result<(), DbError> {
    sqlx::query("DELETE FROM sync_owned_blob_files WHERE path=? AND blob_id IS NULL")
        .bind(path)
        .execute(db)
        .await?;

    Ok(())
}

pub async fn reload_source(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    group: i64,
) -> Result<Option<ObservedPopulatedGroupBundle>, DbError> {
    let mut transaction = db.begin().await?;
    crate::sync_peer::validate_local(&mut transaction, user, identity).await?;
    let value = transport::reload_source(&mut transaction, user, identity, group).await?;
    transaction.commit().await?;

    Ok(value)
}

#[derive(sqlx::FromRow)]
pub struct TransferIntent {
    pub stage_path: String,
    pub token: String,
    pub manifest: String,
    pub directory_device: Option<String>,
    pub directory_inode: Option<String>,
}
pub async fn stage_intents(db: &DbContext) -> Result<Vec<TransferIntent>, DbError> {
    sqlx::query_as("SELECT stage_path,token,manifest,directory_device,directory_inode FROM sync_transfer_intents LIMIT 4097")
        .fetch_all(db)
        .await
}
pub async fn stage_intent(db: &DbContext, path: &str) -> Result<Option<TransferIntent>, DbError> {
    sqlx::query_as("SELECT stage_path,token,manifest,directory_device,directory_inode FROM sync_transfer_intents WHERE stage_path=?")
        .bind(path)
        .fetch_optional(db)
        .await
}
pub async fn publish_stage_manifest(
    db: &DbContext,
    path: &str,
    token: &str,
    manifest: &str,
) -> Result<(), DbError> {
    let result=sqlx::query("UPDATE sync_transfer_intents SET manifest=? WHERE stage_path=? AND token=? AND manifest='[]'").bind(manifest).bind(path).bind(token).execute(db).await?;
    if result.rows_affected() != 1 {
        return Err(failure("identity_blocked"));
    }

    Ok(())
}
pub async fn forget_stage(db: &DbContext, path: &str, token: &str) -> Result<(), DbError> {
    sqlx::query("DELETE FROM sync_transfer_intents WHERE stage_path=? AND token=?")
        .bind(path)
        .bind(token)
        .execute(db)
        .await?;

    Ok(())
}

pub async fn enroll_source(
    db: &DbContext,
    user: &User,
    identity: &SyncIdentity,
    capture: &crate::causal_adoption::PopulatedGroupObservation,
    contents: &[BlobContent],
) -> Result<ObservedPopulatedGroupBundle, DbError> {
    transport::enroll_source(db, user, identity, capture, contents).await
}

/// Accepts only a service-verified HTTPS delivery; this DB boundary is not authentication.
pub async fn begin_delivery(
    db: &DbContext,
    delivery: super::pull::Delivery,
    value: serde_json::Value,
) -> Result<ObservationTransaction, DbError> {
    let plan = parse(&value)?;
    if plan.digest != delivery.digest {
        return Err(failure("identity_blocked"));
    }
    author(&plan, &delivery.issuer.writer_id)?;
    let mut transaction = db.begin_with("BEGIN IMMEDIATE").await?;
    let replay = delivery
        .validate(
            &mut transaction,
            "observed_group",
            transport::batch_id(&plan.bundle),
        )
        .await?;
    let source_echo = source_export(
        &mut transaction,
        &delivery.authorization,
        &delivery.issuer,
        &plan,
    )
    .await?;
    let historical_echo = if source_echo {
        // Originating devices retain the complete checked export digest, but do
        // not have a received-content manifest to reconstruct. Never replace
        // that stronger original proof with a partial received representation.
        true
    } else if delivery
        .previously_applied(
            &mut transaction,
            "observed_group",
            transport::batch_id(&plan.bundle),
        )
        .await?
    {
        if transport::read(
            &mut transaction,
            &delivery.authorization.user,
            transport::batch_id(&plan.bundle),
        )
        .await?
            != value
        {
            return Err(failure("identity_blocked"));
        }
        true
    } else {
        false
    };

    Ok(ObservationTransaction {
        tx: transaction,
        user: delivery.authorization.user.clone(),
        identity: delivery.authorization.identity.clone(),
        origin: delivery.authorization.issuer_origin(&delivery.issuer),
        authority: delivery.issuer.authority_writer_id.clone(),
        writer: delivery.issuer.writer_id.clone(),
        plan,
        prior: replay.then(|| delivery.sequence.to_string()),
        delivery: Some(delivery),
        historical_echo,
    })
}

pub(super) async fn verify_publication(
    db: &mut SqliteConnection,
    authorization: &super::pull::Authorization,
    issuer: &super::pull::Issuer,
    value: &serde_json::Value,
) -> Result<bool, DbError> {
    let plan = parse(value)?;
    author(&plan, &issuer.writer_id)?;

    source_export(db, authorization, issuer, &plan).await
}

async fn source_export(
    db: &mut SqliteConnection,
    authorization: &super::pull::Authorization,
    issuer: &super::pull::Issuer,
    plan: &ObservationPlan,
) -> Result<bool, DbError> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_observation_exports e JOIN causal_observation_batches b USING(user_id,batch_id) WHERE e.user_id=? AND e.batch_id=? AND e.digest=? AND e.server_instance_id=? AND e.server_account_id=? AND e.connection_id=? AND e.authority_writer_id=? AND e.writer_id=? AND b.origin='local' AND b.local_instance_id=? AND b.local_account_id=? AND b.source_instance_id=b.local_instance_id AND b.source_account_id=b.local_account_id)")
        .bind(authorization.user.id).bind(transport::batch_id(&plan.bundle)).bind(&plan.digest).bind(&authorization.server.instance_id).bind(&authorization.server.account_id).bind(&issuer.connection_id).bind(&issuer.authority_writer_id).bind(&issuer.writer_id).bind(&authorization.identity.instance_id).bind(&authorization.identity.account_id).fetch_one(db).await
}

/// Acknowledge an exact historical item without fetching purged content again.
///
/// Original source exports or authenticated receipts must prove the item was
/// already handled. New deliveries never take this path.
pub async fn acknowledge_delivery(
    db: &DbContext,
    delivery: super::pull::Delivery,
    value: serde_json::Value,
) -> Result<Option<Published>, DbError> {
    let transaction = begin_delivery(db, delivery, value).await?;
    if transaction.acknowledgment_only() {
        return transaction.commit(Vec::new()).await.map(Some);
    }
    transaction.tx.rollback().await?;

    Ok(None)
}

/// Record the exact checked source export in the enrollment transaction. Historical
/// unregistered roots remain preserved but cannot gain authority by backfill.
pub(crate) async fn record_source_export(
    db: &mut SqliteConnection,
    user: &User,
    bundle: &ObservedPopulatedGroupBundle,
    contents: Option<&[BlobContent]>,
) -> Result<(), DbError> {
    let Some(contents) = contents else {
        return Ok(());
    };
    let plan = parse(&encode(bundle, contents.to_vec())?)?;
    let writer = &bundle.group_event().writer_id;
    author(&plan, writer)?;
    let prior: Option<String> = sqlx::query_scalar(
        "SELECT digest FROM sync_observation_exports WHERE user_id=? AND batch_id=?",
    )
    .bind(user.id)
    .bind(transport::batch_id(bundle))
    .fetch_optional(&mut *db)
    .await?;
    if let Some(prior) = prior {
        if prior != plan.digest {
            return Err(failure("identity_blocked"));
        }

        return Ok(());
    }
    sqlx::query("INSERT INTO sync_observation_exports(user_id,batch_id,digest,server_instance_id,server_account_id,connection_id,authority_writer_id,writer_id) SELECT a.user_id,?,?,a.server_instance_id,a.server_account_id,a.connection_id,a.authority_writer_id,d.writer_id FROM sync_transport_local_authorities a JOIN local_writer_delegations d ON d.user_id=a.user_id AND d.account_id=a.local_account_id AND d.connection_id=a.connection_id AND d.server_instance_id=a.server_instance_id AND d.server_account_id=a.server_account_id WHERE a.user_id=? AND d.writer_id=?")
        .bind(transport::batch_id(bundle)).bind(plan.digest).bind(user.id).bind(writer).execute(db).await?;

    Ok(())
}
