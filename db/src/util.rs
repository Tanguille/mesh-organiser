use rand::RngExt;

use crate::DbError;

#[must_use]
pub fn random_hex_32() -> String {
    u128_to_hex_32(rand::rng().random::<u128>())
}

/// Formats 128 bits as 32 zero-padded lowercase hex chars (the shape of a global id / blob sha256).
#[must_use]
pub fn u128_to_hex_32(value: u128) -> String {
    format!("{value:032x}")
}

/// Whether `value` has the shape of a stored blob sha256: 32 lowercase hex chars, as
/// [`u128_to_hex_32`] writes them (only the first 128 bits of the hash have ever been kept).
#[must_use]
pub fn is_blob_sha256(value: &str) -> bool {
    value.len() == 32
        && value
            .chars()
            .all(|char| char.is_ascii_digit() || ('a'..='f').contains(&char))
}

#[must_use]
pub fn time_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The caller-supplied `update_timestamp`, falling back to the current time.
#[must_use]
pub fn timestamp_or_now(update_timestamp: Option<&str>) -> String {
    update_timestamp.map_or_else(time_now, ToString::to_string)
}

/// Validates that a unique global id, when given, is exactly 32 characters long. Every `edit_*`
/// entry point checks this before writing anything, so a bad id never leaves a partial edit.
pub fn validate_global_id(id: Option<&str>) -> Result<(), DbError> {
    if id.is_some_and(|id| id.len() != 32) {
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
    fn u128_to_hex_32_pads_leading_zeros() {
        assert_eq!(super::u128_to_hex_32(1), format!("{}1", "0".repeat(31)));
    }

    #[test]
    fn random_hex_32_calls_differ() {
        assert_ne!(super::random_hex_32(), super::random_hex_32());
    }

    #[test]
    fn is_blob_sha256_accepts_only_32_lowercase_hex_chars() {
        for accepted in [
            super::u128_to_hex_32(0),
            super::u128_to_hex_32(u128::MAX),
            super::random_hex_32(),
        ] {
            assert!(super::is_blob_sha256(&accepted), "{accepted:?}");
        }

        for rejected in [
            String::new(),
            "a".repeat(31),
            "a".repeat(33),
            // A full sha256 digest; only the first 128 bits are ever stored.
            "a".repeat(64),
            "A".repeat(32),
            "g".repeat(32),
            // 32 bytes but 31 chars, so a byte-length check alone would let it through.
            format!("{}é", "a".repeat(30)),
            // The blob thumbnail route joins this into a path.
            format!("{}/../etc", "a".repeat(25)),
        ] {
            assert!(!super::is_blob_sha256(&rejected), "{rejected:?}");
        }
    }
}
