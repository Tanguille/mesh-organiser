use rand::RngExt;

use crate::DbError;

#[must_use]
pub fn random_hex_32() -> String {
    format!("{:032x}", rand::rng().random::<u128>())
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

#[cfg(test)]
mod tests {
    #[test]
    fn random_hex_32_is_32_lowercase_hex_chars() {
        let hex = super::random_hex_32();

        assert_eq!(hex.len(), 32);
        assert!(
            hex.chars()
                .all(|char| char.is_ascii_digit() || ('a'..='f').contains(&char)),
            "expected only [0-9a-f], got {hex}"
        );
    }

    #[test]
    fn random_hex_32_calls_differ() {
        assert_ne!(super::random_hex_32(), super::random_hex_32());
    }
}
