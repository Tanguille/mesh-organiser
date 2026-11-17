//! Certified staging and recoverable final links. No cleanup follows caller paths.
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use rustix::fs::{AtFlags, FileType, Mode, OFlags};

use super::{AppState, ServiceError, invalid, observations};

#[cfg(unix)]
mod slots;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Link {
    name: String,
    device: u64,
    inode: u64,
}

pub(super) struct PublishedFile {
    pub path: PathBuf,
    pub device: u64,
    pub inode: u64,
}

#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)] // Other platforms reject unprovable inode ownership.
fn identity(metadata: &std::fs::Metadata) -> Result<(u64, u64), ServiceError> {
    Ok((metadata.dev(), metadata.ino()))
}
#[cfg(not(unix))]
fn identity(_metadata: &std::fs::Metadata) -> Result<(u64, u64), ServiceError> {
    Err(invalid("unsupported"))
}

pub(super) fn sync_directory(path: &Path) -> Result<(), ServiceError> {
    #[cfg(unix)]
    {
        sync_handle(&open_directory(path)?, path)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(invalid("unsupported"))
    }
}
#[cfg(unix)]
fn sync_handle(directory: &File, path: &Path) -> Result<(), ServiceError> {
    directory.sync_all()?;
    #[cfg(test)]
    super::crash_tests::directory_synced(path)?;
    #[cfg(not(test))]
    let _ = path;

    Ok(())
}

fn text(path: &Path) -> Result<&str, ServiceError> {
    path.to_str().ok_or_else(|| invalid("identity_blocked"))
}
fn root(state: &AppState) -> Result<PathBuf, ServiceError> {
    Ok(std::fs::canonicalize(state.get_model_dir())?)
}
fn owned_child(parent: &Path, path: &Path, prefix: &str) -> bool {
    path.parent() == Some(parent)
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(prefix))
}

#[cfg(unix)]
fn open_directory(path: &Path) -> Result<File, ServiceError> {
    Ok(rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(std::io::Error::from)?)
}
#[cfg(not(unix))]
fn open_directory(_path: &Path) -> Result<File, ServiceError> {
    Err(invalid("unsupported"))
}

#[cfg(unix)]
fn open_lease(directory: &File, create: bool) -> Result<File, ServiceError> {
    let mut flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    if create {
        flags |= OFlags::CREATE | OFlags::EXCL;
    }
    Ok(
        rustix::fs::openat(directory, ".lease", flags, Mode::RUSR | Mode::WUSR)
            .map(File::from)
            .map_err(std::io::Error::from)?,
    )
}
#[cfg(not(unix))]
fn open_lease(_directory: &File, _create: bool) -> Result<File, ServiceError> {
    Err(invalid("unsupported"))
}

fn verify_directory(
    path: &Path,
    directory: &File,
    intent: &observations::TransferIntent,
) -> Result<(), ServiceError> {
    let expected = (
        intent
            .directory_device
            .as_deref()
            .and_then(|directory_device| directory_device.parse::<u64>().ok()),
        intent
            .directory_inode
            .as_deref()
            .and_then(|directory_inode| directory_inode.parse::<u64>().ok()),
    );
    let actual = identity(&directory.metadata()?)?;
    if expected != (Some(actual.0), Some(actual.1))
        || identity(&std::fs::symlink_metadata(path)?)? != actual
    {
        return Err(invalid("identity_blocked"));
    }
    Ok(())
}

/// Never remove a directory by pathname: POSIX has no inode-conditional rmdir.
/// Clear through the verified descriptor and retain the lease until the durable
/// intent is forgotten. Unexpected subdirectories are retained, never traversed.
#[cfg(unix)]
fn clear_directory(path: &Path, directory: &File) -> Result<(), ServiceError> {
    let expected = identity(&directory.metadata()?)?;
    if identity(&std::fs::symlink_metadata(path)?)? != expected {
        return Err(invalid("identity_blocked"));
    }
    let entries = rustix::fs::Dir::read_from(directory).map_err(std::io::Error::from)?;
    for entry in entries {
        let entry = entry.map_err(std::io::Error::from)?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b".." | b".lease") {
            continue;
        }
        rustix::fs::unlinkat(directory, name, AtFlags::empty()).map_err(std::io::Error::from)?;
    }
    directory.sync_all()?;
    if identity(&std::fs::symlink_metadata(path)?)? != expected {
        return Err(invalid("identity_blocked"));
    }
    // Leave the empty name rather than race a replacement after the last check.
    // Certified slot reservations bound creation and allow later safe reuse.
    #[cfg(test)]
    super::crash_tests::checkpoint("before_stage_directory_remove")?;
    Ok(())
}
#[cfg(not(unix))]
fn clear_directory(_path: &Path, _directory: &File) -> Result<(), ServiceError> {
    Err(invalid("unsupported"))
}

#[derive(Debug)]
pub(super) struct Stage {
    path: PathBuf,
    handle: File,
    // The descriptor stays locked until after cleanup; other processes skip it.
    lease: File,
    token: String,
    preserve: bool,
}
impl Drop for Stage {
    fn drop(&mut self) {
        if !self.preserve {
            let _ = clear_directory(&self.path, &self.handle);
        }
    }
}
impl Stage {
    pub(super) async fn new(state: &AppState) -> Result<Self, ServiceError> {
        // Directory durability/inode ownership is deliberately unsupported elsewhere.
        sync_directory(&root(state)?)?;
        recover(state).await?;
        #[cfg(unix)]
        {
            slots::acquire(state).await
        }
        #[cfg(not(unix))]
        {
            Err(invalid("unsupported"))
        }
    }
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
    pub(super) fn retain(&mut self) -> Result<(), ServiceError> {
        sync_directory(self.path())?;
        // Set before the DB commit: cancellation cannot remove a directory whose
        // durable outbox reference may already have become visible.
        self.preserve = true;
        Ok(())
    }
    pub(super) fn token(&self) -> &str {
        &self.token
    }
    pub(super) async fn resume(
        state: &AppState,
        path: &str,
        token: &str,
    ) -> Result<Self, ServiceError> {
        let root = root(state)?;
        let path = PathBuf::from(path);
        if !owned_child(&root, &path, ".sync-stage-") {
            return Err(invalid("identity_blocked"));
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || !std::fs::symlink_metadata(path.join(".lease"))?.is_file() {
            return Err(invalid("identity_blocked"));
        }
        let handle = open_directory(&path)?;
        let lease = open_lease(&handle, false)?;
        lease.try_lock().map_err(|_| invalid("publication_busy"))?;
        #[cfg(unix)]
        slots::verify(state, &path, &handle, &lease).await?;
        let mut actual = String::new();
        (&lease).take(65).read_to_string(&mut actual)?;
        let intent = observations::stage_intent(&state.db, text(&path)?)
            .await?
            .ok_or_else(|| invalid("identity_blocked"))?;
        verify_directory(&path, &handle, &intent)?;
        if actual != token
            || intent.token != token
            || intent.manifest != "[]"
            || identity(&metadata)? != identity(&std::fs::symlink_metadata(&path)?)?
            || !db::sync_transport::outbox::stage_retained(&state.db, text(&path)?, token).await?
        {
            return Err(invalid("identity_blocked"));
        }
        Ok(Self {
            handle,
            path,
            lease,
            token: token.into(),
            preserve: true,
        })
    }
    pub(super) async fn publish(
        &mut self,
        state: &AppState,
        contents: &[super::BlobContent],
        raw: &BTreeMap<String, PathBuf>,
    ) -> Result<BTreeMap<String, PublishedFile>, ServiceError> {
        let root = root(state)?;
        let mut links = Vec::new();
        let mut files = Vec::new();
        let mut result = BTreeMap::new();
        for content in contents {
            let source = raw
                .get(&content.sha256)
                .ok_or_else(|| invalid("identity_blocked"))?;
            let stored = self.path().join(format!("stored-{}", content.sha256));
            super::store_file(source, content, &stored).await?;
            let (device, inode) = identity(&std::fs::symlink_metadata(&stored)?)?;
            let name = format!(".sync-blob-{}.{}", db::random_hex_32(), content.filetype);
            let target = root.join(&name);
            links.push(Link {
                name,
                device,
                inode,
            });
            files.push((stored, target.clone()));
            result.insert(
                content.hash.clone(),
                PublishedFile {
                    path: target,
                    device,
                    inode,
                },
            );
        }
        sync_directory(self.path())?;
        // Preserve the certificate even if an async cancellation follows its commit.
        self.preserve = true;
        observations::publish_stage_manifest(
            &state.db,
            text(self.path())?,
            &self.token,
            &serde_json::to_string(&links)?,
        )
        .await?;
        for (source, target) in files {
            std::fs::hard_link(source, target)?;
        }
        #[cfg(all(test, unix))]
        super::crash_tests::checkpoint("before_final_directory_sync")?;
        sync_directory(&root)?;
        #[cfg(all(test, unix))]
        super::crash_tests::checkpoint("files_published")?;

        Ok(result)
    }
    pub(super) async fn finish(&mut self, state: &AppState) -> Result<(), ServiceError> {
        let path = self.path().to_path_buf();
        if db::sync_transport::outbox::stage_retained(&state.db, text(&path)?, &self.token).await? {
            return Err(invalid("publication_pending"));
        }
        let intent = observations::stage_intent(&state.db, text(&path)?)
            .await?
            .ok_or_else(|| invalid("identity_blocked"))?;
        if intent.token != self.token {
            return Err(invalid("identity_blocked"));
        }
        verify_directory(&path, &self.handle, &intent)?;
        #[cfg(unix)]
        slots::verify(state, &path, &self.handle, &self.lease).await?;
        cleanup_links(state, &intent.manifest, &path, &self.handle).await?;
        // On Unix the lease remains held even after unlinking its directory entry.
        clear_directory(&path, &self.handle)?;
        sync_directory(&root(state)?)?;
        #[cfg(all(test, unix))]
        super::crash_tests::checkpoint("stage_contents_cleared")?;
        observations::forget_stage(&state.db, text(&path)?, &self.token).await?;
        // The permanent certified lease remains for a later generation.
        self.preserve = false;

        Ok(())
    }
}
fn missing(path: &Path) -> Result<bool, ServiceError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}
/// Capture without replacing any quarantine entry, using the leased directory
/// descriptor throughout. Path replacement must never redirect cleanup.
#[cfg(unix)]
async fn cleanup_links(
    state: &AppState,
    manifest: &str,
    directory: &Path,
    handle: &File,
) -> Result<(), ServiceError> {
    if manifest.len() > 1024 * 1024 {
        return Err(invalid("too_large"));
    }
    let links: Vec<Link> = serde_json::from_str(manifest)?;
    if links.len() > db::causal_envelope::MAX_CONTEXT_ITEMS {
        return Err(invalid("too_large"));
    }
    let root = root(state)?;
    let root_handle = open_directory(&root)?;
    for (index, link) in links.into_iter().enumerate() {
        let path = root.join(&link.name);
        if !owned_child(&root, &path, ".sync-blob-") || link.name.contains(['/', '\\']) {
            return Err(invalid("identity_blocked"));
        }
        let quarantine = format!(".cleanup-{index}");
        let committed = observations::path_committed(&state.db, text(&path)?).await?;
        if entry_identity(handle, &quarantine)?.is_none() {
            if committed {
                continue;
            }
            if entry_identity(&root_handle, &link.name)?
                != Some((FileType::RegularFile, link.device, link.inode))
            {
                continue;
            }
            #[cfg(test)]
            super::crash_tests::checkpoint("before_orphan_capture")?;
            // Recheck after the asynchronous DB lookup and before taking ownership.
            // Atomic no-replace also protects a destination created after this check.
            verify_path_handle(directory, handle)?;
            verify_path_handle(&root, &root_handle)?;
            capture_no_replace(&root_handle, &link.name, handle, &quarantine)?;
            sync_handle(handle, directory)?;
            sync_handle(&root_handle, &root)?;
        }
        let captured = entry_identity(handle, &quarantine)?;
        if committed || captured != Some((FileType::RegularFile, link.device, link.inode)) {
            restore_quarantine(handle, &quarantine, &root_handle, &link.name, &root)?;
        } else {
            rustix::fs::unlinkat(handle, &quarantine, AtFlags::empty())
                .map_err(std::io::Error::from)?;
        }
        sync_handle(handle, directory)?;
        sync_handle(&root_handle, &root)?;
    }

    Ok(())
}
#[cfg(not(unix))]
async fn cleanup_links(
    _state: &AppState,
    _manifest: &str,
    _directory: &Path,
    _handle: &File,
) -> Result<(), ServiceError> {
    Err(invalid("unsupported"))
}

#[cfg(unix)]
fn verify_path_handle(path: &Path, handle: &File) -> Result<(), ServiceError> {
    if identity(&std::fs::symlink_metadata(path)?)? != identity(&handle.metadata()?)? {
        return Err(invalid("identity_blocked"));
    }

    Ok(())
}

#[cfg(unix)]
#[allow(clippy::unnecessary_cast)] // Native dev_t/ino_t widths differ across Unix targets.
fn entry_identity(
    directory: &File,
    name: &str,
) -> Result<Option<(FileType, u64, u64)>, ServiceError> {
    match rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(Some((
            FileType::from_raw_mode(stat.st_mode),
            stat.st_dev as u64,
            stat.st_ino as u64,
        ))),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(std::io::Error::from(error).into()),
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
fn capture_no_replace(
    root: &File,
    name: &str,
    directory: &File,
    quarantine: &str,
) -> Result<(), ServiceError> {
    rustix::fs::renameat_with(
        root,
        name,
        directory,
        quarantine,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(std::io::Error::from)?;

    Ok(())
}
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_vendor = "apple"))
))]
fn capture_no_replace(
    _root: &File,
    _name: &str,
    _directory: &File,
    _quarantine: &str,
) -> Result<(), ServiceError> {
    // Never fall back to an overwrite-capable rename on unsupported systems.
    Err(invalid("unsupported"))
}

#[cfg(unix)]
fn restore_quarantine(
    directory: &File,
    quarantine: &str,
    root: &File,
    name: &str,
    root_path: &Path,
) -> Result<(), ServiceError> {
    // Never overwrite a newer replacement. If both differ, retain both files and
    // the durable intent for a later recovery attempt instead of losing data.
    match rustix::fs::linkat(directory, quarantine, root, name, AtFlags::empty()) {
        Ok(()) => {}
        Err(rustix::io::Errno::EXIST) => {
            if entry_identity(root, name)? != entry_identity(directory, quarantine)? {
                return Err(invalid("identity_blocked"));
            }
        }
        Err(error) => return Err(std::io::Error::from(error).into()),
    }
    // Preserve the already-durable quarantine until the public link is durable.
    // A failed sync leaves both names and the intent for a later retry.
    #[cfg(test)]
    super::crash_tests::checkpoint("before_restore_directory_sync")?;
    sync_handle(root, root_path)?;
    #[cfg(test)]
    super::crash_tests::checkpoint("before_restore_unlink")?;
    rustix::fs::unlinkat(directory, quarantine, AtFlags::empty()).map_err(std::io::Error::from)?;
    directory.sync_all()?;

    Ok(())
}

pub(super) async fn remove_orphan(
    state: &AppState,
    orphan: &observations::OwnedBlobFile,
) -> Result<(), ServiceError> {
    let root = root(state)?;
    let path = Path::new(&orphan.path);
    if !owned_child(&root, path, ".sync-blob-") {
        return Err(invalid("identity_blocked"));
    }
    let expected = (
        orphan
            .device
            .parse::<u64>()
            .map_err(|_| invalid("identity_blocked"))?,
        orphan
            .inode
            .parse::<u64>()
            .map_err(|_| invalid("identity_blocked"))?,
    );
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && identity(&metadata)? == expected => {}
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            observations::forget_orphan_path(&state.db, &orphan.path).await?;

            return Ok(());
        }
        Err(error) => return Err(error.into()),
    }
    let mut staging = Stage::new(state).await?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("identity_blocked"))?
        .to_owned();
    let manifest = serde_json::to_string(&vec![Link {
        name,
        device: expected.0,
        inode: expected.1,
    }])?;
    staging.preserve = true;
    observations::publish_stage_manifest(
        &state.db,
        text(staging.path())?,
        &staging.token,
        &manifest,
    )
    .await?;
    staging.finish(state).await?;
    // A replacement that won the race remains at its original path, with evidence.
    if missing(path)? {
        observations::forget_orphan_path(&state.db, &orphan.path).await?;
    }

    Ok(())
}

pub(super) async fn recover(state: &AppState) -> Result<(), ServiceError> {
    let intents = observations::stage_intents(&state.db).await?;
    if intents.is_empty() {
        return Ok(());
    }
    let root = root(state)?;
    sync_directory(&root)?;
    if intents.len() > 4096 {
        return Err(invalid("quota_paused"));
    }
    for intent in intents {
        // Historical intents have no physical database/lease proof. Preserve them
        // as blocked capacity rather than inventing a certificate after a clone.
        let Some(slot) =
            db::sync_transport::stage_slots::get(&state.db, &intent.stage_path).await?
        else {
            continue;
        };
        // A copied database retains the original storage certificate as blocked
        // capacity. It may allocate its own slot, but cannot clean the old one.
        #[cfg(unix)]
        if !slots::owned_by(state, &slot).await? {
            continue;
        }
        #[cfg(not(unix))]
        let _ = slot;
        let path = Path::new(&intent.stage_path);
        if !owned_child(&root, path, ".sync-stage-") {
            return Err(invalid("identity_blocked"));
        }
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() => metadata,
            Ok(_) => return Err(invalid("identity_blocked")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Missing private storage cannot authorize deleting a public path.
                // Normal completed cleanup also reaches this state before clearing its row.
                observations::forget_stage(&state.db, &intent.stage_path, &intent.token).await?;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let lease_path = path.join(".lease");
        if !std::fs::symlink_metadata(&lease_path)?.is_file() {
            return Err(invalid("identity_blocked"));
        }
        let handle = open_directory(path)?;
        verify_directory(path, &handle, &intent)?;
        let lease = open_lease(&handle, false)?;
        match lease.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => continue,
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        #[cfg(unix)]
        slots::verify(state, path, &handle, &lease).await?;
        let mut token = String::new();
        (&lease).take(65).read_to_string(&mut token)?;
        if token != intent.token
            || identity(&metadata)? != identity(&std::fs::symlink_metadata(path)?)?
        {
            return Err(invalid("identity_blocked"));
        }
        // Re-read after acquiring the lease; a previous owner may have published
        // its manifest after our initial query and then exited.
        let Some(latest) = observations::stage_intent(&state.db, &intent.stage_path).await? else {
            continue;
        };
        if latest.token != token {
            return Err(invalid("identity_blocked"));
        }
        verify_directory(path, &handle, &latest)?;
        #[cfg(all(test, unix))]
        super::crash_tests::checkpoint("outbox_before_retention_check")?;
        if db::sync_transport::outbox::stage_retained(&state.db, &intent.stage_path, &token).await?
        {
            continue;
        }
        cleanup_links(state, &latest.manifest, path, &handle).await?;
        clear_directory(path, &handle)?;
        sync_directory(&root)?;
        #[cfg(all(test, unix))]
        super::crash_tests::checkpoint("stage_contents_cleared")?;
        observations::forget_stage(&state.db, &intent.stage_path, &token).await?;
        // Keep the permanent lease and certified directory for safe reuse.
    }

    Ok(())
}
