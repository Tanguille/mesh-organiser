//! Upper bounds for query-string-derived allocations (IDs and UTF-8 strings).
//!
//! Limits are expressed in UTF-8 bytes for strings (`str::len()`).

use thiserror::Error;

/// Maximum number of `i64` IDs accepted per repeated query parameter (`?model_ids=1&model_ids=2`).
pub const MAX_ID_LIST_ITEMS: usize = 10_000;

/// Maximum UTF-8 byte length for `text_search`.
pub const MAX_QUERY_TEXT_BYTES: usize = 8192;

/// Maximum UTF-8 byte length for `order_by`.
pub const MAX_ORDER_BY_BYTES: usize = 256;

/// Maximum UTF-8 byte length for the raw `model_ids_str` query value before splitting.
pub const MAX_MODEL_IDS_STR_BYTES: usize = 256 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum QueryBoundsError {
    #[error("Too many values in {field}; maximum allowed is {max}")]
    TooManyIds { field: &'static str, max: usize },
    #[error("{field} exceeds maximum length of {max} bytes (UTF-8)")]
    StringTooLong { field: &'static str, max: usize },
    #[error("Too many model_ids in model_ids_str; maximum allowed is {max}")]
    TooManyCommaSeparatedIds { max: usize },
}

/// Rejects `ids` if it contains more than [`MAX_ID_LIST_ITEMS`] elements.
pub const fn validate_id_list(field: &'static str, ids: &[i64]) -> Result<(), QueryBoundsError> {
    if ids.len() > MAX_ID_LIST_ITEMS {
        return Err(QueryBoundsError::TooManyIds {
            field,
            max: MAX_ID_LIST_ITEMS,
        });
    }

    Ok(())
}

/// Validates `model_ids`, `group_ids`, and `label_ids` list lengths.
pub fn validate_three_id_lists(
    model_ids: &[i64],
    group_ids: &[i64],
    label_ids: &[i64],
) -> Result<(), QueryBoundsError> {
    validate_id_list("model_ids", model_ids)?;
    validate_id_list("group_ids", group_ids)?;
    validate_id_list("label_ids", label_ids)?;

    Ok(())
}

/// Rejects `opt` if `Some` and UTF-8 byte length exceeds `max_bytes`.
pub const fn validate_optional_str(
    str: Option<&str>,
    field: &'static str,
    max_bytes: usize,
) -> Result<(), QueryBoundsError> {
    if let Some(s) = str
        && s.len() > max_bytes
    {
        return Err(QueryBoundsError::StringTooLong {
            field,
            max: max_bytes,
        });
    }

    Ok(())
}

/// Parses comma-separated decimal `i64` values; invalid segments are skipped.
/// Returns [`QueryBoundsError::TooManyCommaSeparatedIds`] if more than `max` values parse successfully.
pub fn parse_comma_separated_i64(str: &str, max: usize) -> Result<Vec<i64>, QueryBoundsError> {
    let mut out = Vec::new();
    for part in str.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Ok(id) = part.parse::<i64>() {
            if out.len() >= max {
                return Err(QueryBoundsError::TooManyCommaSeparatedIds { max });
            }
            out.push(id);
        }
    }

    Ok(out)
}

/// Validates optional raw `model_ids_str` byte length.
pub const fn validate_model_ids_str_raw(str: Option<&str>) -> Result<(), QueryBoundsError> {
    validate_optional_str(str, "model_ids_str", MAX_MODEL_IDS_STR_BYTES)
}

/// Validates shared list + string fields for model and group list endpoints.
pub fn validate_list_query_strings(
    text_search: Option<&str>,
    order_by: Option<&str>,
) -> Result<(), QueryBoundsError> {
    validate_optional_str(text_search, "text_search", MAX_QUERY_TEXT_BYTES)?;
    validate_optional_str(order_by, "order_by", MAX_ORDER_BY_BYTES)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_comma_empty_and_invalid_skipped() {
        assert_eq!(
            parse_comma_separated_i64("", 10).unwrap(),
            Vec::<i64>::new()
        );
        assert_eq!(
            parse_comma_separated_i64(",,,", 10).unwrap(),
            Vec::<i64>::new()
        );
        assert_eq!(parse_comma_separated_i64("1,x,2", 10).unwrap(), vec![1, 2]);
    }

    #[test]
    fn parse_comma_respects_max_successful() {
        let s = "1,2,3";
        assert_eq!(parse_comma_separated_i64(s, 3).unwrap(), vec![1, 2, 3]);
        assert_eq!(
            parse_comma_separated_i64(s, 2),
            Err(QueryBoundsError::TooManyCommaSeparatedIds { max: 2 })
        );
    }

    #[test]
    fn validate_id_list_rejects_over_limit() {
        let ids = vec![0i64; MAX_ID_LIST_ITEMS + 1];
        assert!(validate_id_list("model_ids", &ids).is_err());
    }

    #[test]
    fn validate_optional_str_rejects_long() {
        let s = "a".repeat(MAX_QUERY_TEXT_BYTES + 1);
        assert!(validate_optional_str(Some(&s), "text_search", MAX_QUERY_TEXT_BYTES).is_err());
    }

    #[test]
    fn validate_three_id_lists_and_strings_ok_for_empty() {
        assert!(validate_three_id_lists(&[], &[], &[]).is_ok());
        assert!(validate_list_query_strings(None, None).is_ok());
    }
}
