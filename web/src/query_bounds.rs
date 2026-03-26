//! Upper bounds for query-string-derived allocations (IDs and UTF-8 strings).
//!
//! Limits are expressed in UTF-8 bytes for strings (`str::len()`).

use std::str::FromStr;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;

use db::{MAX_PAGE_SIZE, group_db::GroupOrderBy, model_db::ModelOrderBy};

/// Maximum number of `i64` IDs accepted per repeated query parameter (`?model_ids=1&model_ids=2`).
pub const MAX_ID_LIST_ITEMS: usize = 10_000;

/// Maximum `page` index (1-based) for list endpoints; keeps offset arithmetic bounded.
pub const MAX_PAGE: u32 = 1_000_000;

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
    #[error("page must be between 1 and {max} inclusive")]
    PageOutOfRange { max: u32 },
    #[error("page_size must be between 1 and {max} inclusive")]
    PageSizeOutOfRange { max: u32 },
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

fn validate_id_lists_and_list_strings(
    model_ids: &[i64],
    group_ids: &[i64],
    label_ids: &[i64],
    text_search: Option<&str>,
    order_by: Option<&str>,
) -> Result<(), QueryBoundsError> {
    validate_three_id_lists(model_ids, group_ids, label_ids)?;
    validate_list_query_strings(text_search, order_by)?;

    Ok(())
}

/// Shared query fields for paginated model/group list endpoints.
#[derive(Clone, Copy)]
pub struct PaginatedListQueryBounds<'a> {
    pub model_ids: &'a [i64],
    pub group_ids: &'a [i64],
    pub label_ids: &'a [i64],
    pub text_search: Option<&'a str>,
    pub order_by: Option<&'a str>,
    pub page: u32,
    pub page_size: u32,
}

/// Full query bounds for paginated model list endpoints (`GET /models`, share variants).
pub fn validate_model_list_query_bounds(
    query_bounds: PaginatedListQueryBounds<'_>,
) -> Result<(), QueryBoundsError> {
    validate_id_lists_and_list_strings(
        query_bounds.model_ids,
        query_bounds.group_ids,
        query_bounds.label_ids,
        query_bounds.text_search,
        query_bounds.order_by,
    )?;
    validate_pagination(query_bounds.page, query_bounds.page_size)?;

    Ok(())
}

/// Full query bounds for paginated group list endpoints (`GET /groups`, share variants).
pub fn validate_group_list_query_bounds(
    query_bounds: PaginatedListQueryBounds<'_>,
    model_ids_str: Option<&str>,
) -> Result<(), QueryBoundsError> {
    validate_id_lists_and_list_strings(
        query_bounds.model_ids,
        query_bounds.group_ids,
        query_bounds.label_ids,
        query_bounds.text_search,
        query_bounds.order_by,
    )?;
    validate_model_ids_str_raw(model_ids_str)?;
    validate_pagination(query_bounds.page, query_bounds.page_size)?;

    Ok(())
}

/// HTTP 400 response for [`QueryBoundsError`] (for handlers that `return Ok(...)`).
#[must_use]
pub fn bad_request(e: &QueryBoundsError) -> Response {
    (StatusCode::BAD_REQUEST, e.to_string()).into_response()
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

/// Parses comma-separated `model_ids_str` when present; [`None`] when the query param is absent.
pub fn optional_comma_separated_model_ids(
    str: Option<&str>,
) -> Result<Option<Vec<i64>>, QueryBoundsError> {
    str.map_or(Ok(None), |s| {
        parse_comma_separated_i64(s, MAX_ID_LIST_ITEMS).map(Some)
    })
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

/// Parses `order_by` query values only when UTF-8 length is within [`MAX_ORDER_BY_BYTES`], so
/// `FromStr` work stays bounded even if validation is skipped by mistake.
#[must_use]
pub fn parse_model_order_by_bounded(str: &str) -> ModelOrderBy {
    if str.len() > MAX_ORDER_BY_BYTES {
        return ModelOrderBy::AddedDesc;
    }

    ModelOrderBy::from_str(str).unwrap_or(ModelOrderBy::AddedDesc)
}

/// Same as [`parse_model_order_by_bounded`] for group list `order_by`.
#[must_use]
pub fn parse_group_order_by_bounded(str: &str) -> GroupOrderBy {
    if str.len() > MAX_ORDER_BY_BYTES {
        return GroupOrderBy::NameAsc;
    }

    GroupOrderBy::from_str(str).unwrap_or(GroupOrderBy::NameAsc)
}

/// Rejects pagination parameters that would allow unbounded allocations or overflow in offset math.
pub fn validate_pagination(page: u32, page_size: u32) -> Result<(), QueryBoundsError> {
    if !(1..=MAX_PAGE).contains(&page) {
        return Err(QueryBoundsError::PageOutOfRange { max: MAX_PAGE });
    }

    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(QueryBoundsError::PageSizeOutOfRange { max: MAX_PAGE_SIZE });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use db::{MAX_PAGE_SIZE, group_db::GroupOrderBy, model_db::ModelOrderBy};

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

    #[test]
    fn validate_pagination_accepts_typical_range() {
        assert!(validate_pagination(1, 50).is_ok());
        assert!(validate_pagination(MAX_PAGE, MAX_PAGE_SIZE).is_ok());
    }

    #[test]
    fn validate_pagination_rejects_page_zero() {
        assert_eq!(
            validate_pagination(0, 50),
            Err(QueryBoundsError::PageOutOfRange { max: MAX_PAGE })
        );
    }

    #[test]
    fn validate_pagination_rejects_oversized_page() {
        assert_eq!(
            validate_pagination(MAX_PAGE + 1, 50),
            Err(QueryBoundsError::PageOutOfRange { max: MAX_PAGE })
        );
    }

    #[test]
    fn validate_pagination_rejects_page_size_zero_or_over_limit() {
        assert_eq!(
            validate_pagination(1, 0),
            Err(QueryBoundsError::PageSizeOutOfRange { max: MAX_PAGE_SIZE })
        );
        assert_eq!(
            validate_pagination(1, MAX_PAGE_SIZE + 1),
            Err(QueryBoundsError::PageSizeOutOfRange { max: MAX_PAGE_SIZE })
        );
    }

    #[test]
    fn validate_model_list_query_bounds_matches_individual_validators() {
        let ok = PaginatedListQueryBounds {
            model_ids: &[],
            group_ids: &[],
            label_ids: &[],
            text_search: None,
            order_by: None,
            page: 1,
            page_size: 50,
        };
        assert!(validate_model_list_query_bounds(ok).is_ok());
        assert_eq!(
            validate_model_list_query_bounds(PaginatedListQueryBounds { page: 0, ..ok }),
            Err(QueryBoundsError::PageOutOfRange { max: MAX_PAGE })
        );
    }

    #[test]
    fn validate_group_list_query_bounds_includes_model_ids_str() {
        let base = PaginatedListQueryBounds {
            model_ids: &[],
            group_ids: &[],
            label_ids: &[],
            text_search: None,
            order_by: None,
            page: 1,
            page_size: 50,
        };
        assert!(validate_group_list_query_bounds(base, None).is_ok());
        let long = "a".repeat(MAX_MODEL_IDS_STR_BYTES + 1);
        assert_eq!(
            validate_group_list_query_bounds(base, Some(&long)),
            Err(QueryBoundsError::StringTooLong {
                field: "model_ids_str",
                max: MAX_MODEL_IDS_STR_BYTES,
            })
        );
    }

    #[test]
    fn optional_comma_separated_model_ids_none_and_some() {
        assert_eq!(optional_comma_separated_model_ids(None).unwrap(), None);
        assert_eq!(
            optional_comma_separated_model_ids(Some("1,2")).unwrap(),
            Some(vec![1, 2])
        );
    }

    #[test]
    fn parse_model_order_by_bounded_rejects_oversized_without_parsing() {
        let junk = "AddedDesc".repeat(100);
        assert!(junk.len() > MAX_ORDER_BY_BYTES);
        assert_eq!(parse_model_order_by_bounded(&junk), ModelOrderBy::AddedDesc);
    }

    #[test]
    fn parse_model_order_by_bounded_accepts_known_variant() {
        assert_eq!(
            parse_model_order_by_bounded("NameAsc"),
            ModelOrderBy::NameAsc
        );
    }

    #[test]
    fn parse_group_order_by_bounded_rejects_oversized() {
        let junk = "NameAsc".repeat(100);
        assert!(junk.len() > MAX_ORDER_BY_BYTES);
        assert_eq!(parse_group_order_by_bounded(&junk), GroupOrderBy::NameAsc);
    }

    #[test]
    fn parse_group_order_by_bounded_accepts_known_variant() {
        assert_eq!(
            parse_group_order_by_bounded("CreatedDesc"),
            GroupOrderBy::CreatedDesc
        );
    }
}
