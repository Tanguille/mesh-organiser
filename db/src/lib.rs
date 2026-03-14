// Pedantic doc lints (missing_errors_doc, missing_panics_doc) are allowed here;
// db modules are internal and error semantics are consistent (DbError from SQLite).
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod blob_db;
pub mod db_context;
pub mod group_db;
pub mod label_db;
pub mod label_keyword_db;
pub mod model;
pub mod model_db;
mod paginated_response;
pub mod resource_db;
pub mod share_db;
pub mod user_db;
pub use paginated_response::PaginatedResponse;
mod util;

pub use util::{random_hex_32, time_now};

pub type DbError = sqlx::Error;
