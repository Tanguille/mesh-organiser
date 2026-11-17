//! Experimental authenticated hub exchange. No production route activates this API.

use crate::DbError;

mod auth;
mod exchange;
pub mod observations;
mod offline;
pub mod operation_selection;
pub mod outbox;
pub mod pairing;
pub mod publication;
pub mod pull;
pub mod stage_slots;

pub use auth::{activate_writer, register_writer};
pub use exchange::{Page, Published, apply_delivery, list, publish};
pub(crate) use offline::rotate_all;
pub use offline::{
    OfflineWriterProof, StoredAuthority, install_authority, load_authority, writer_proof,
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct WriterRegistration {
    pub server_identity: crate::user_db::SyncIdentity,
    pub connection_id: String,
    pub writer_id: String,
    pub writer_secret: String,
}

impl std::fmt::Debug for WriterRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriterRegistration")
            .field("server_identity", &self.server_identity)
            .field("connection_id", &self.connection_id)
            .field("writer_id", &self.writer_id)
            .finish_non_exhaustive()
    }
}

pub const MAX_WIRE_ITEM_BYTES: usize = 1024 * 1024;
pub const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PAGE_ITEMS: usize = 64;
pub(crate) fn failure(reason: &str) -> DbError {
    DbError::Protocol(format!("sync_transport:{reason}"))
}

pub fn encode_genesis(
    bundle: &crate::causal_genesis::GenesisBundle,
) -> Result<serde_json::Value, DbError> {
    crate::causal_genesis::transport::encode_group(bundle)
}
pub fn encode_operation(
    bundle: &crate::causal_group_lifecycle::OperationBundle,
) -> Result<serde_json::Value, DbError> {
    crate::causal_group_lifecycle::transport::encode_operation(bundle)
}
