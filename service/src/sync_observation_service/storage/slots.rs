//! Permanent certified directories; generation tokens fence their active intents.
use super::{
    AppState, File, Path, ServiceError, Stage, entry_identity, identity, invalid, open_directory,
    open_lease, root, sync_handle, text, verify_path_handle,
};
use db::sync_transport::stage_slots::{self, StageSlot};
use std::io::{Seek, SeekFrom, Write};

async fn owner(state: &AppState) -> Result<StageSlot, ServiceError> {
    let root = root(state)?;
    let (root_device, root_inode) = identity(&std::fs::metadata(&root)?)?;
    let database = std::fs::canonicalize(stage_slots::database_path(&state.db).await?)?;
    let (database_device, database_inode) = identity(&std::fs::metadata(&database)?)?;

    Ok(StageSlot {
        stage_path: String::new(),
        root_path: text(&root)?.to_owned(),
        root_device: root_device.to_string(),
        root_inode: root_inode.to_string(),
        database_path: text(&database)?.to_owned(),
        database_device: database_device.to_string(),
        database_inode: database_inode.to_string(),
        directory_device: None,
        directory_inode: None,
        lease_device: None,
        lease_inode: None,
    })
}
fn same_owner(slot: &StageSlot, owner: &StageSlot) -> bool {
    slot.root_path == owner.root_path
        && slot.root_device == owner.root_device
        && slot.root_inode == owner.root_inode
        && slot.database_path == owner.database_path
        && slot.database_device == owner.database_device
        && slot.database_inode == owner.database_inode
}
pub(super) async fn owned_by(state: &AppState, slot: &StageSlot) -> Result<bool, ServiceError> {
    Ok(same_owner(slot, &owner(state).await?))
}
fn certificate(actual: (u64, u64), device: Option<&str>, inode: Option<&str>) -> bool {
    device == Some(actual.0.to_string().as_str()) && inode == Some(actual.1.to_string().as_str())
}
pub(super) async fn verify(
    state: &AppState,
    path: &Path,
    handle: &File,
    lease: &File,
) -> Result<StageSlot, ServiceError> {
    let slot = stage_slots::get(&state.db, text(path)?)
        .await?
        .ok_or_else(|| invalid("identity_blocked"))?;
    if !same_owner(&slot, &owner(state).await?) {
        return Err(invalid("identity_blocked"));
    }
    verify_files(&slot, path, handle, lease)?;

    Ok(slot)
}

fn verify_files(
    slot: &StageSlot,
    path: &Path,
    handle: &File,
    lease: &File,
) -> Result<(), ServiceError> {
    let root_identity = identity(&std::fs::metadata(&slot.root_path)?)?;
    let database_identity = identity(&std::fs::metadata(&slot.database_path)?)?;
    if slot.root_device != root_identity.0.to_string()
        || slot.root_inode != root_identity.1.to_string()
        || slot.database_device != database_identity.0.to_string()
        || slot.database_inode != database_identity.1.to_string()
        || !certificate(
            identity(&handle.metadata()?)?,
            slot.directory_device.as_deref(),
            slot.directory_inode.as_deref(),
        )
        || !certificate(
            identity(&lease.metadata()?)?,
            slot.lease_device.as_deref(),
            slot.lease_inode.as_deref(),
        )
        || entry_identity(handle, ".lease")?
            != Some((
                rustix::fs::FileType::RegularFile,
                identity(&lease.metadata()?)?.0,
                identity(&lease.metadata()?)?.1,
            ))
    {
        return Err(invalid("identity_blocked"));
    }
    verify_path_handle(path, handle)?;

    Ok(())
}
fn empty_payload(handle: &File) -> Result<bool, ServiceError> {
    for entry in rustix::fs::Dir::read_from(handle).map_err(std::io::Error::from)? {
        let entry = entry.map_err(std::io::Error::from)?;
        if !matches!(entry.file_name().to_bytes(), b"." | b".." | b".lease") {
            return Ok(false);
        }
    }

    Ok(true)
}
async fn activate(
    state: &AppState,
    slot: &StageSlot,
    handle: File,
    mut lease: File,
) -> Result<Option<Stage>, ServiceError> {
    verify(state, Path::new(&slot.stage_path), &handle, &lease).await?;
    let token = db::random_hex_32();
    let mut tx = state.db.begin_with("BEGIN IMMEDIATE").await?;
    if !stage_slots::activate_in(&mut tx, slot, &token).await? {
        return Ok(None);
    }
    if !empty_payload(&handle)? {
        return Err(invalid("identity_blocked"));
    }
    // All DB checks above use this transaction; taking another pooled connection
    // while holding the writer would deadlock a saturated or single-slot pool.
    verify_files(slot, Path::new(&slot.stage_path), &handle, &lease)?;
    lease.rewind()?;
    lease.set_len(0)?;
    lease.write_all(token.as_bytes())?;
    lease.sync_all()?;
    lease.seek(SeekFrom::Start(0))?;
    // This owner exists before COMMIT can be dispatched. Cancellation releases
    // its lease but cannot clear a generation whose commit may still complete.
    let mut reservation = Stage {
        path: slot.stage_path.clone().into(),
        handle,
        lease,
        token,
        preserve: true,
    };
    tx.commit().await?;
    reservation.preserve = false;

    Ok(Some(reservation))
}

pub(super) async fn acquire(state: &AppState) -> Result<Stage, ServiceError> {
    use rustix::fs::{Mode, OFlags};
    let mut owner = owner(state).await?;
    for slot in stage_slots::list(&state.db).await? {
        if !same_owner(&slot, &owner) || slot.directory_device.is_none() {
            continue;
        }
        let path = Path::new(&slot.stage_path);
        let Ok(handle) = open_directory(path) else {
            continue;
        };
        let Ok(lease) = open_lease(&handle, false) else {
            continue;
        };
        if lease.try_lock().is_err() || verify(state, path, &handle, &lease).await.is_err() {
            continue;
        }
        if let Some(stage) = activate(state, &slot, handle, lease).await? {
            return Ok(stage);
        }
    }
    let root_path = std::path::PathBuf::from(&owner.root_path);
    let root_handle = open_directory(&root_path)?;
    let name = format!(".sync-stage-{}", db::random_hex_32());
    text(&root_path.join(&name))?.clone_into(&mut owner.stage_path);
    stage_slots::reserve(&state.db, &owner).await?;
    #[cfg(test)]
    super::super::crash_tests::checkpoint("stage_slot_reserved")?;
    verify_path_handle(&root_path, &root_handle)?;
    rustix::fs::mkdirat(&root_handle, &name, Mode::RWXU).map_err(std::io::Error::from)?;
    let handle = File::from(
        rustix::fs::openat(
            &root_handle,
            &name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?,
    );
    let lease = open_lease(&handle, true)?;
    lease.lock()?;
    lease.sync_all()?;
    sync_handle(&handle, Path::new(&owner.stage_path))?;
    sync_handle(&root_handle, &root_path)?;
    #[cfg(test)]
    super::super::crash_tests::checkpoint("stage_slot_created")?;
    let (device, inode) = identity(&handle.metadata()?)?;
    owner.directory_device = Some(device.to_string());
    owner.directory_inode = Some(inode.to_string());
    let (device, inode) = identity(&lease.metadata()?)?;
    owner.lease_device = Some(device.to_string());
    owner.lease_inode = Some(inode.to_string());
    verify_path_handle(&root_path, &root_handle)?;
    verify_path_handle(Path::new(&owner.stage_path), &handle)?;
    stage_slots::certify(&state.db, &owner).await?;

    activate(state, &owner, handle, lease)
        .await?
        .ok_or_else(|| invalid("identity_blocked"))
}
