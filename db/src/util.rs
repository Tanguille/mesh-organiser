use rand::RngExt;

use crate::DbError;

#[must_use]
pub fn random_hex_32() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill(&mut bytes);
    hex::encode(bytes)
}

#[must_use]
pub fn time_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Validates that a unique global id is exactly 32 characters long, the invariant shared by
/// every `edit_*_global_id` entry point.
pub fn validate_global_id(id: &str) -> Result<(), DbError> {
    if id.len() != 32 {
        return Err(DbError::InvalidArgument(
            "Unique Global ID must be 32 characters long".to_string(),
        ));
    }

    Ok(())
}

/// Parses a `GROUP_CONCAT` csv string into a `Vec<i64>`, skipping any segments that don't parse.
#[must_use]
pub fn parse_concat_ids(csv: &str) -> Vec<i64> {
    csv.split(',')
        .filter_map(|segment| segment.parse::<i64>().ok())
        .collect()
}
