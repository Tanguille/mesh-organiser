//! Bounded physical-content transfer. No production sync route enables this API.
use std::{
    collections::BTreeMap,
    fmt::Write,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use storage::Stage;
use tokio::{
    fs::File,
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
};
use tokio_util::compat::TokioAsyncReadCompatExt;

use db::{
    causal_envelope::Dot,
    model::{blob::Blob, user::User},
    sync_transport::observations::{self, BlobContent},
    user_db::SyncIdentity,
};

use crate::{AppState, ServiceError, export_service};

mod storage;

pub const MAX_OBSERVATION_BLOB_BYTES: u64 = observations::MAX_BLOB_BYTES;
pub const MAX_OBSERVATION_TOTAL_BYTES: u64 = observations::MAX_TOTAL_BYTES;
/// Bound untrusted staging independently from the ordinary import lock. The
/// permit is retained through detached finalization, even when the client leaves.
fn staging_permit(state: &AppState) -> Result<tokio::sync::OwnedSemaphorePermit, ServiceError> {
    type Slots = std::sync::Mutex<BTreeMap<PathBuf, std::sync::Weak<tokio::sync::Semaphore>>>;
    static DIRECTORIES: std::sync::LazyLock<Slots> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(BTreeMap::new()));
    let path = std::fs::canonicalize(state.get_model_dir())?;
    let mut directories = DIRECTORIES.lock().map_err(|_| invalid("quota_paused"))?;
    directories.retain(|_, slots| slots.strong_count() > 0);
    let slots = directories
        .get(&path)
        .and_then(std::sync::Weak::upgrade)
        .unwrap_or_else(|| {
            let capacity = std::thread::available_parallelism()
                .map_or(1, std::num::NonZeroUsize::get)
                .clamp(2, 4);
            let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(capacity));
            directories.insert(path, std::sync::Arc::downgrade(&slots));
            slots
        });

    slots
        .try_acquire_owned()
        .map_err(|_| invalid("quota_paused"))
}
fn invalid(reason: &str) -> ServiceError {
    db::DbError::Protocol(format!("sync_transport:{reason}")).into()
}
fn digest_hex(digest: Sha256) -> String {
    digest
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");

            output
        })
}
async fn stage(state: &AppState) -> Result<Stage, ServiceError> {
    Stage::new(state).await
}
#[derive(Debug)]
pub struct PreparedBlob {
    key: String,
    path: PathBuf,
    size: u64,
}
impl PreparedBlob {
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }
}
#[derive(Debug)]
pub struct PreparedObservedGroup {
    item: serde_json::Value,
    blobs: Vec<PreparedBlob>,
    group_event: Dot,
    // The staged snapshot must outlive upload, including changes to the source file.
    stage: Stage,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl PreparedObservedGroup {
    #[must_use]
    pub const fn item(&self) -> &serde_json::Value {
        &self.item
    }
    #[must_use]
    pub fn blobs(&self) -> &[PreparedBlob] {
        &self.blobs
    }
    #[must_use]
    pub const fn group_event(&self) -> &Dot {
        &self.group_event
    }
    pub(crate) async fn finish_queued(&mut self, state: &AppState) -> Result<(), ServiceError> {
        self.stage.finish(state).await
    }
}

pub(crate) async fn queue_observed_group(
    state: &AppState,
    user: &User,
    identity: &SyncIdentity,
    authorization: &db::sync_transport::pull::Authorization,
    group_id: i64,
) -> Result<String, ServiceError> {
    if !authorization.matches(user, identity) {
        return Err(invalid("identity_blocked"));
    }
    if let Some(id) =
        db::sync_transport::outbox::selected_observation(&state.db, authorization, group_id).await?
    {
        return Ok(id);
    }
    let _permit = staging_permit(state)?;
    let _lock = state.import_mutex.lock().await;
    let SourcePreparation {
        mut directory,
        blobs,
        contents,
        capture,
        ..
    } = prepare_source(state, user, identity, group_id).await?;
    let record = source_stage_record(&directory, &blobs)?;
    directory.retain()?;
    #[cfg(all(test, unix))]
    crash_tests::checkpoint("outbox_files_synced")?;
    let result = db::sync_transport::outbox::queue_observation(
        &state.db,
        authorization,
        group_id,
        capture.as_ref(),
        &contents,
        &record,
    )
    .await;
    match result {
        Ok((id, retained)) => {
            #[cfg(all(test, unix))]
            crash_tests::checkpoint("outbox_committed")?;
            if !retained {
                directory.finish(state).await?;
            }
            Ok(id)
        }
        Err(error) => {
            let _ = directory.finish(state).await;
            Err(error.into())
        }
    }
}

pub(crate) async fn resume_queued(
    state: &AppState,
    queued: &db::sync_transport::outbox::Queued,
) -> Result<PreparedObservedGroup, ServiceError> {
    let permit = staging_permit(state)?;
    let record = queued
        .stage
        .as_ref()
        .ok_or_else(|| invalid("identity_blocked"))?;
    let directory = Stage::resume(state, &record.path, &record.token).await?;
    let plan = observations::parse(&queued.item)?;
    if record.files.len() != plan.contents().len() {
        return Err(invalid("identity_blocked"));
    }
    let mut names = std::collections::BTreeSet::new();
    let mut blobs = Vec::new();
    for content in plan.contents() {
        let file = record
            .files
            .iter()
            .find(|file| file.key == content.sha256)
            .ok_or_else(|| invalid("identity_blocked"))?;
        if file
            .name
            .parse::<usize>()
            .ok()
            .is_none_or(|item| item.to_string() != file.name)
            || !names.insert(&file.name)
            || u64::try_from(content.size) != Ok(file.size)
        {
            return Err(invalid("identity_blocked"));
        }
        let path = directory.path().join(&file.name);
        if !tokio::fs::symlink_metadata(&path).await?.is_file() {
            return Err(invalid("identity_blocked"));
        }
        let mut reader = File::open(&path).await?.take(file.size + 1);
        let mut hash = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = [0_u8; 8192];
        loop {
            let bytes_read = reader.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            hash.update(&buffer[..bytes_read]);
            size += u64::try_from(bytes_read).map_err(|_| invalid("too_large"))?;
        }
        if size != file.size || digest_hex(hash) != file.key {
            return Err(invalid("identity_blocked"));
        }
        blobs.push(PreparedBlob {
            key: file.key.clone(),
            path,
            size,
        });
    }
    Ok(PreparedObservedGroup {
        item: queued.item.clone(),
        blobs,
        group_event: queued.event.clone(),
        stage: directory,
        _permit: permit,
    })
}

pub(crate) async fn discard_queued(
    state: &AppState,
    user: &User,
    identity: &SyncIdentity,
    id: &str,
) -> Result<(), ServiceError> {
    let record = db::sync_transport::outbox::discard_stage(&state.db, user, identity, id).await?;
    let mut directory = if let Some(record) = &record {
        Some(Stage::resume(state, &record.path, &record.token).await?)
    } else {
        None
    };
    let state = state.clone();
    let user = user.clone();
    let identity = identity.clone();
    let id = id.to_owned();
    // A selected cancellation cannot retract an already transmitted operation.
    // The lease only serializes local publication/cleanup of this owned stage.
    tokio::spawn(async move {
        db::sync_transport::outbox::discard(&state.db, &user, &identity, &id, record.as_ref())
            .await?;
        if let Some(directory) = &mut directory {
            directory.finish(&state).await?;
        }
        Ok::<_, ServiceError>(())
    })
    .await
    .map_err(|_| invalid("identity_blocked"))??;
    Ok(())
}

#[cfg(all(test, unix))]
pub(crate) fn outbox_acknowledged() -> Result<(), ServiceError> {
    crash_tests::checkpoint("outbox_acknowledged")?;
    Ok(())
}

/// Copies bounded logical bytes, including decompressed content, into an owned snapshot.
async fn copy_verified(
    mut reader: impl AsyncRead + Unpin,
    path: &Path,
    size: i64,
    prefix: &str,
    full: Option<&str>,
) -> Result<String, ServiceError> {
    let expected = u64::try_from(size).map_err(|_| invalid("identity_blocked"))?;
    if expected > MAX_OBSERVATION_BLOB_BYTES {
        return Err(invalid("too_large"));
    }
    let mut file = File::create_new(path).await?;
    let mut digest = Sha256::new();
    let mut count = 0_u64;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        count += u64::try_from(read).map_err(|_| invalid("too_large"))?;
        if count > expected {
            return Err(invalid("identity_blocked"));
        }
        digest.update(&buffer[..read]);
        file.write_all(&buffer[..read]).await?;
    }
    let hash = digest_hex(digest);
    if count != expected
        || hash.get(..32) != Some(prefix)
        || full.is_some_and(|value| value != hash)
    {
        return Err(invalid("identity_blocked"));
    }
    file.sync_all().await?;

    Ok(hash)
}
fn source_stage_record(
    directory: &Stage,
    blobs: &[PreparedBlob],
) -> Result<db::sync_transport::outbox::StageRecord, ServiceError> {
    Ok(db::sync_transport::outbox::StageRecord {
        path: directory
            .path()
            .to_str()
            .ok_or_else(|| invalid("identity_blocked"))?
            .into(),
        token: directory.token().into(),
        files: blobs
            .iter()
            .map(|blob| {
                Ok(db::sync_transport::outbox::StageFile {
                    name: blob
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or_else(|| invalid("identity_blocked"))?
                        .into(),
                    key: blob.key.clone(),
                    size: blob.size,
                })
            })
            .collect::<Result<_, ServiceError>>()?,
    })
}

pub(crate) struct PreparedIdentitySource {
    source: SourcePreparation,
    record: db::causal_genesis::identity::LocalObservationStage,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

pub(crate) async fn prepare_identity_source(
    state: &AppState,
    user: &User,
    identity: &SyncIdentity,
    group_id: i64,
) -> Result<PreparedIdentitySource, ServiceError> {
    let permit = staging_permit(state)?;
    let source = prepare_source(state, user, identity, group_id).await?;
    if source.prior.is_some() || source.capture.is_none() {
        return Err(invalid("identity_blocked"));
    }
    let record = db::causal_genesis::identity::LocalObservationStage {
        contents: source.contents.clone(),
        stage: source_stage_record(&source.directory, &source.blobs)?,
    };

    Ok(PreparedIdentitySource {
        source,
        record,
        _permit: permit,
    })
}

struct SourcePreparation {
    directory: Stage,
    blobs: Vec<PreparedBlob>,
    contents: Vec<BlobContent>,
    capture: Option<db::causal_adoption::PopulatedGroupObservation>,
    prior: Option<db::causal_adoption::ObservedPopulatedGroupBundle>,
}
async fn prepare_source(
    state: &AppState,
    user: &User,
    identity: &SyncIdentity,
    group_id: i64,
) -> Result<SourcePreparation, ServiceError> {
    let prior = observations::reload_source(&state.db, user, identity, group_id).await?;
    let capture = if prior.is_none() {
        Some(
            db::causal_adoption::observe_populated_group(&state.db, user, identity, group_id)
                .await?,
        )
    } else {
        None
    };
    let descriptors = observations::source_blobs(&state.db, user, group_id).await?;
    let directory = stage(state).await?;
    let mut blobs = Vec::new();
    let mut contents = Vec::new();
    let mut total = 0_u64;
    for (index, blob) in descriptors.into_iter().enumerate() {
        let size = u64::try_from(blob.size).map_err(|_| invalid("identity_blocked"))?;
        total = total.saturating_add(size);
        if total > MAX_OBSERVATION_TOTAL_BYTES {
            return Err(invalid("too_large"));
        }
        let path = directory.path().join(index.to_string());
        let reader = export_service::open_blob_content_reader(&blob, state).await?;
        let sha256 = copy_verified(reader, &path, blob.size, &blob.sha256, None).await?;
        blobs.push(PreparedBlob {
            key: sha256.clone(),
            path,
            size,
        });
        contents.push(BlobContent {
            hash: blob.sha256,
            filetype: blob.filetype,
            size: blob.size,
            sha256,
        });
    }
    Ok(SourcePreparation {
        directory,
        blobs,
        contents,
        capture,
        prior,
    })
}

pub async fn prepare_observed_group(
    state: &AppState,
    user: &User,
    identity: &SyncIdentity,
    group_id: i64,
) -> Result<PreparedObservedGroup, ServiceError> {
    let permit = staging_permit(state)?;
    let _lock = state.import_mutex.lock().await;
    let SourcePreparation {
        directory,
        blobs,
        contents,
        capture,
        prior,
    } = prepare_source(state, user, identity, group_id).await?;
    // Enrollment rechecks every captured identity/epoch after the potentially slow file reads.
    let bundle = if let Some(capture) = capture {
        observations::enroll_source(&state.db, user, identity, &capture, &contents).await?
    } else {
        let bundle = observations::reload_source(&state.db, user, identity, group_id)
            .await?
            .ok_or_else(|| invalid("identity_blocked"))?;
        if prior
            .as_ref()
            .is_none_or(|prior| prior.group_event() != bundle.group_event())
        {
            return Err(invalid("identity_blocked"));
        }
        bundle
    };
    let item = observations::encode(&bundle, contents)?;

    Ok(PreparedObservedGroup {
        item,
        blobs,
        group_event: bundle.group_event().clone(),
        stage: directory,
        _permit: permit,
    })
}

/// Incoming files live only in an app-owned temporary directory until the entire body verifies.
pub struct IncomingObservation {
    directory: Stage,
    _permit: tokio::sync::OwnedSemaphorePermit,
    contents: Vec<BlobContent>,
    received: BTreeMap<String, PathBuf>,
}
pub struct IncomingBlob {
    file: File,
    path: PathBuf,
    content: BlobContent,
    digest: Sha256,
    count: u64,
}
impl IncomingObservation {
    pub async fn new(
        state: &AppState,
        plan: &observations::ObservationPlan,
    ) -> Result<Self, ServiceError> {
        let permit = staging_permit(state)?;
        Ok(Self {
            directory: stage(state).await?,
            _permit: permit,
            contents: plan.contents().to_vec(),
            received: BTreeMap::new(),
        })
    }
    pub async fn start_blob(&self, key: &str) -> Result<IncomingBlob, ServiceError> {
        let content = self
            .contents
            .iter()
            .find(|value| value.sha256 == key)
            .ok_or_else(|| invalid("identity_blocked"))?
            .clone();
        if self.received.contains_key(key) {
            return Err(invalid("identity_blocked"));
        }
        // The filename comes from the validated fixed-length digest, never multipart filenames.
        let path = self.directory.path().join(key);
        let file = File::create_new(&path).await?;

        Ok(IncomingBlob {
            file,
            path,
            content,
            digest: Sha256::new(),
            count: 0,
        })
    }
    pub async fn finish_blob(&mut self, blob: IncomingBlob) -> Result<(), ServiceError> {
        if blob.count
            != u64::try_from(blob.content.size).map_err(|_| invalid("identity_blocked"))?
            || digest_hex(blob.digest) != blob.content.sha256
        {
            return Err(invalid("identity_blocked"));
        }
        blob.file.sync_all().await?;
        storage::sync_directory(self.directory.path())?;
        self.received.insert(blob.content.sha256, blob.path);
        #[cfg(all(test, unix))]
        crash_tests::checkpoint("stage_written")?;

        Ok(())
    }
    pub fn complete(&self) -> Result<(), ServiceError> {
        if self.received.len() != self.contents.len() {
            return Err(invalid("identity_blocked"));
        }

        Ok(())
    }
}
impl IncomingBlob {
    pub async fn write(&mut self, bytes: &[u8]) -> Result<(), ServiceError> {
        self.count = self
            .count
            .saturating_add(u64::try_from(bytes.len()).map_err(|_| invalid("too_large"))?);
        if self.count > MAX_OBSERVATION_BLOB_BYTES {
            return Err(invalid("too_large"));
        }
        if self.count > u64::try_from(self.content.size).map_err(|_| invalid("identity_blocked"))? {
            return Err(invalid("too_large"));
        }
        self.digest.update(bytes);
        self.file.write_all(bytes).await?;

        Ok(())
    }
}

#[allow(clippy::suspicious_operation_groupings)] // Native blob.sha256 is the legacy 128-bit prefix.
async fn verify_existing(
    state: &AppState,
    blob: &Blob,
    content: &BlobContent,
    directory: &Path,
) -> Result<(), ServiceError> {
    if blob.sha256 != content.hash || blob.filetype != content.filetype || blob.size != content.size
    {
        return Err(invalid("identity_blocked"));
    }
    let reader = export_service::open_blob_content_reader(blob, state).await?;
    copy_verified(
        reader,
        &directory.join(format!("verify-{}", content.sha256)),
        content.size,
        &content.hash,
        Some(&content.sha256),
    )
    .await?;

    Ok(())
}
async fn store_file(raw: &Path, content: &BlobContent, path: &Path) -> Result<(), ServiceError> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut output = options.open(path).await?;
    let mut input = File::open(raw).await?;
    if let Some(extension) = content.filetype.strip_suffix(".zip") {
        let mut archive = async_zip::tokio::write::ZipFileWriter::with_tokio(&mut output);
        let entry = async_zip::ZipEntryBuilder::new(
            format!("model.{extension}").into(),
            async_zip::Compression::Deflate,
        );
        let mut writer = archive.write_entry_stream(entry).await?;
        futures::io::copy(&mut input.compat(), &mut writer).await?;
        writer.close().await?;
        crate::util::finish_zip(archive).await?;
    } else {
        tokio::io::copy(&mut input, &mut output).await?;
    }
    output.sync_all().await?;

    Ok(())
}

/// Called in an owned task after the request body is complete. A disconnect may lose
/// the ACK, but must not cancel between committing metadata and preserving its file.
#[allow(clippy::too_many_arguments)]
pub async fn install_received(
    state: AppState,
    user: User,
    account: String,
    connection: String,
    secret: String,
    nonce: Option<String>,
    item: serde_json::Value,
    incoming: IncomingObservation,
) -> Result<db::sync_transport::Published, ServiceError> {
    install_target(
        state,
        user,
        CommitTarget::Upload {
            account,
            connection,
            secret,
            nonce,
        },
        item,
        incoming,
    )
    .await?
    .publication()
}
enum CommitTarget {
    Upload {
        account: String,
        connection: String,
        secret: String,
        nonce: Option<String>,
    },
    Delivery(Box<db::sync_transport::pull::Delivery>),
    Identity {
        delivery: Box<db::sync_transport::pull::Delivery>,
        decision_id: String,
        decision: db::causal_genesis::identity::Decision,
        local_source: Option<db::causal_genesis::identity::LocalObservationStage>,
    },
}
pub(crate) async fn install_delivery(
    state: AppState,
    user: User,
    delivery: db::sync_transport::pull::Delivery,
    item: serde_json::Value,
    incoming: IncomingObservation,
) -> Result<db::sync_transport::Published, ServiceError> {
    install_target(
        state,
        user,
        CommitTarget::Delivery(Box::new(delivery)),
        item,
        incoming,
    )
    .await?
    .publication()
}
enum InstallOutcome {
    Publication(db::sync_transport::Published),
    Identity(db::causal_genesis::identity::DecisionOutcome),
}
impl InstallOutcome {
    fn publication(self) -> Result<db::sync_transport::Published, ServiceError> {
        match self {
            Self::Publication(result) => Ok(result),
            Self::Identity(_) => Err(invalid("identity_blocked")),
        }
    }
}

#[allow(clippy::too_many_arguments)] // Both verified stages must remain owned until the one decision commit.
pub(crate) async fn install_identity_delivery(
    state: AppState,
    user: User,
    delivery: db::sync_transport::pull::Delivery,
    decision_id: String,
    decision: db::causal_genesis::identity::Decision,
    item: serde_json::Value,
    incoming: IncomingObservation,
    mut local_source: Option<PreparedIdentitySource>,
) -> Result<db::causal_genesis::identity::DecisionOutcome, ServiceError> {
    if let Some(local) = &mut local_source {
        local.source.directory.retain()?;
    }
    let result = install_target(
        state.clone(),
        user,
        CommitTarget::Identity {
            delivery: Box::new(delivery),
            decision_id,
            decision,
            local_source: local_source.as_ref().map(|local| local.record.clone()),
        },
        item,
        incoming,
    )
    .await;
    if let Some(local) = &mut local_source
        && !db::sync_transport::outbox::stage_retained(
            &state.db,
            &local.record.stage.path,
            &local.record.stage.token,
        )
        .await?
    {
        let cleanup = local.source.directory.finish(&state).await;
        if result.is_ok() {
            cleanup?;
        }
    }
    match result? {
        InstallOutcome::Identity(result) => Ok(result),
        InstallOutcome::Publication(_) => Err(invalid("identity_blocked")),
    }
}

async fn commit_install(
    tx: observations::ObservationTransaction,
    bindings: Vec<(String, i64)>,
    identity: Option<(
        String,
        db::causal_genesis::identity::Decision,
        Option<db::causal_genesis::identity::LocalObservationStage>,
    )>,
) -> Result<InstallOutcome, ServiceError> {
    match identity {
        Some((decision_id, decision, local_source)) => Ok(InstallOutcome::Identity(
            Box::pin(tx.commit_identity(&decision_id, decision, bindings, local_source.as_ref()))
                .await?,
        )),
        None => Ok(InstallOutcome::Publication(tx.commit(bindings).await?)),
    }
}

async fn install_target(
    state: AppState,
    user: User,
    target: CommitTarget,
    item: serde_json::Value,
    mut incoming: IncomingObservation,
) -> Result<InstallOutcome, ServiceError> {
    incoming.complete()?;
    let _lock = state.import_mutex.lock().await;

    // Keep the owned transfer finalizer off the task stack as decision validation grows.
    let result = Box::pin(async {
        let paths = incoming
            .directory
            .publish(&state, &incoming.contents, &incoming.received)
            .await?;
        let (mut tx, identity) = match target {
            CommitTarget::Upload {
                account,
                connection,
                secret,
                nonce,
            } => (
                observations::begin(
                    &state.db,
                    &user,
                    &account,
                    &connection,
                    &secret,
                    nonce.as_deref(),
                    item,
                )
                .await?,
                None,
            ),
            CommitTarget::Delivery(delivery) => (
                observations::begin_delivery(&state.db, *delivery, item).await?,
                None,
            ),
            CommitTarget::Identity {
                delivery,
                decision_id,
                decision,
                local_source,
            } => (
                observations::begin_delivery(&state.db, *delivery, item).await?,
                Some((decision_id, decision, local_source)),
            ),
        };
        if tx.contents() != incoming.contents {
            return Err(invalid("identity_blocked"));
        }
        if tx.acknowledgment_only() {
            return commit_install(tx, Vec::new(), identity).await;
        }
        let mut bindings = Vec::new();
        for content in &incoming.contents {
            let id = if let Some(blob) = tx.existing_blob(&content.hash).await? {
                verify_existing(&state, &blob, content, incoming.directory.path()).await?;
                blob.id
            } else {
                let path = paths
                    .get(&content.hash)
                    .ok_or_else(|| invalid("identity_blocked"))?;
                let text = path
                    .path
                    .to_str()
                    .ok_or_else(|| invalid("identity_blocked"))?;
                tx.create_blob(content, text, path.device, path.inode)
                    .await?
            };
            bindings.push((content.hash.clone(), id));
        }
        let published = commit_install(tx, bindings, identity).await?;
        #[cfg(all(test, unix))]
        crash_tests::checkpoint("receipt_committed")?;
        Ok(published)
    })
    .await;
    let cleanup = incoming.directory.finish(&state).await;
    if result.is_ok() {
        cleanup?;
    }

    result
}

pub struct VerifiedDownload {
    file: File,
    pub size: u64,
    _directory: Stage,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl AsyncRead for VerifiedDownload {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.file).poll_read(context, buffer)
    }
}
pub async fn prepare_download(
    state: &AppState,
    content: &BlobContent,
    blob: &Blob,
) -> Result<VerifiedDownload, ServiceError> {
    let permit = staging_permit(state)?;
    let _lock = state.import_mutex.lock().await;
    let directory = stage(state).await?;
    verify_existing(state, blob, content, directory.path()).await?;
    let file = File::open(directory.path().join(format!("verify-{}", content.sha256))).await?;

    Ok(VerifiedDownload {
        file,
        size: u64::try_from(content.size).map_err(|_| invalid("identity_blocked"))?,
        _directory: directory,
        _permit: permit,
    })
}

/// The caller holds the import lock. Only persisted app-owned orphans are eligible;
/// failure keeps their ledger entry so a later sweep can retry.
pub(crate) async fn remove_owned_orphans(state: &AppState) -> Result<(), ServiceError> {
    storage::recover(state).await?;
    for orphan in observations::orphan_paths(&state.db).await? {
        storage::remove_orphan(state, &orphan).await?;
    }

    Ok(())
}

pub async fn recover_transfers(state: &AppState) -> Result<(), ServiceError> {
    let _lock = state.import_mutex.lock().await;
    storage::recover(state).await
}

#[cfg(all(test, unix))]
#[path = "sync_observation_service/crash_tests.rs"]
mod crash_tests;
