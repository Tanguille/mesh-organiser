pub mod blob_db;
pub mod causal_adoption;
mod causal_clock;
pub mod causal_envelope;
pub mod causal_genesis;
pub mod causal_group_lifecycle;
pub mod causal_group_rename;
pub mod causal_model_move;
pub mod causal_store;
pub mod db_context;
pub mod group_db;
pub mod group_history;
mod group_sync;
pub mod label_db;
pub mod label_keyword_db;
pub mod model;
pub mod model_db;
mod paginated_response;
mod query_util;
pub mod resource_db;
pub mod share_db;
pub mod sync_peer;
pub mod sync_snapshot;
pub mod sync_transport;
pub mod user_db;
pub use paginated_response::PaginatedResponse;
mod util;

pub(crate) use query_util::{
    GROUPS, LABELS, MODELS, RESOURCES, ensure_owned, push_in, set_timestamp_column,
};

pub use util::{is_blob_sha256, random_hex_32, time_now, u128_to_hex_32};
pub(crate) use util::{parse_concat_ids, timestamp_or_now, validate_global_id};

pub type DbError = sqlx::Error;

/// Maximum page size to prevent memory exhaustion and unbounded queries
pub const MAX_PAGE_SIZE: u32 = 1000;
