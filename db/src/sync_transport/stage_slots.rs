//! Bounded physical storage reservations. Filesystem proofs are checked by service.
use super::failure;
use crate::{DbError, db_context::DbContext};
use sqlx::SqliteConnection;

pub const MAX_STAGE_SLOTS: i64 = 4096;

pub async fn database_path(db: &DbContext) -> Result<String, DbError> {
    let mut connection = db.acquire().await?;
    let databases: Vec<(i64, String, String)> = sqlx::query_as("PRAGMA database_list")
        .fetch_all(&mut *connection)
        .await?;
    let mut handle = connection.lock_handle().await?;
    let mut moved: std::ffi::c_int = 1;
    // SAFETY: sqlx locks out its worker while this handle lives; SQLite receives
    // a valid connection, a static NUL-terminated name, and a writable c_int.
    let result = unsafe {
        libsqlite3_sys::sqlite3_file_control(
            handle.as_raw_handle().as_ptr(),
            c"main".as_ptr(),
            libsqlite3_sys::SQLITE_FCNTL_HAS_MOVED,
            std::ptr::from_mut(&mut moved).cast(),
        )
    };
    if result != libsqlite3_sys::SQLITE_OK || moved != 0 {
        return Err(failure("identity_blocked"));
    }
    // shortcut: this rejects detected moves, but does not prove inode ownership
    // across pathname ABA or pooled connections; sync stays paused until it can.

    databases
        .into_iter()
        .find(|(_, name, path)| name == "main" && !path.is_empty())
        .map(|(_, _, path)| path)
        .ok_or_else(|| failure("unsupported"))
}

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct StageSlot {
    pub stage_path: String,
    pub root_path: String,
    pub root_device: String,
    pub root_inode: String,
    pub database_path: String,
    pub database_device: String,
    pub database_inode: String,
    pub directory_device: Option<String>,
    pub directory_inode: Option<String>,
    pub lease_device: Option<String>,
    pub lease_inode: Option<String>,
}

pub async fn list(db: &DbContext) -> Result<Vec<StageSlot>, DbError> {
    sqlx::query_as("SELECT * FROM sync_stage_slots ORDER BY stage_path LIMIT 4097")
        .fetch_all(db)
        .await
}

pub async fn get(db: &DbContext, path: &str) -> Result<Option<StageSlot>, DbError> {
    sqlx::query_as("SELECT * FROM sync_stage_slots WHERE stage_path=?")
        .bind(path)
        .fetch_optional(db)
        .await
}

pub async fn reserve(db: &DbContext, slot: &StageSlot) -> Result<(), DbError> {
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let count: i64 = sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM sync_stage_slots)+(SELECT COUNT(*) FROM sync_transfer_intents i WHERE NOT EXISTS(SELECT 1 FROM sync_stage_slots s WHERE s.stage_path=i.stage_path))")
        .fetch_one(&mut *tx).await?;
    if count >= MAX_STAGE_SLOTS {
        return Err(failure("quota_paused"));
    }
    sqlx::query("INSERT INTO sync_stage_slots(stage_path,root_path,root_device,root_inode,database_path,database_device,database_inode) VALUES(?,?,?,?,?,?,?)")
        .bind(&slot.stage_path).bind(&slot.root_path).bind(&slot.root_device).bind(&slot.root_inode)
        .bind(&slot.database_path).bind(&slot.database_device).bind(&slot.database_inode)
        .execute(&mut *tx).await?;
    tx.commit().await?;

    Ok(())
}

pub async fn certify(db: &DbContext, slot: &StageSlot) -> Result<(), DbError> {
    let changed = sqlx::query("UPDATE sync_stage_slots SET directory_device=?,directory_inode=?,lease_device=?,lease_inode=? WHERE stage_path=? AND directory_device IS NULL")
        .bind(&slot.directory_device).bind(&slot.directory_inode).bind(&slot.lease_device).bind(&slot.lease_inode).bind(&slot.stage_path)
        .execute(db).await?.rows_affected();
    if changed != 1 {
        return Err(failure("identity_blocked"));
    }

    Ok(())
}

/// Called under the permanent OS lease and an IMMEDIATE transaction. The caller
/// fsyncs the new lease token before committing this active generation.
pub async fn activate_in(
    tx: &mut SqliteConnection,
    slot: &StageSlot,
    token: &str,
) -> Result<bool, DbError> {
    let current: Option<StageSlot> =
        sqlx::query_as("SELECT * FROM sync_stage_slots WHERE stage_path=?")
            .bind(&slot.stage_path)
            .fetch_optional(&mut *tx)
            .await?;
    if current.as_ref() != Some(slot) || slot.directory_device.is_none() {
        return Err(failure("identity_blocked"));
    }
    let occupied: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sync_transfer_intents WHERE stage_path=?) OR EXISTS(SELECT 1 FROM sync_publication_stages WHERE stage_path=?)")
        .bind(&slot.stage_path).bind(&slot.stage_path).fetch_one(&mut *tx).await?;
    if occupied {
        return Ok(false);
    }
    sqlx::query("INSERT INTO sync_transfer_intents(stage_path,token,directory_device,directory_inode) VALUES(?,?,?,?)")
        .bind(&slot.stage_path).bind(token).bind(&slot.directory_device).bind(&slot.directory_inode)
        .execute(tx).await?;

    Ok(true)
}
