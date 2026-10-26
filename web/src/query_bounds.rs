//! Upper bounds for query-string-derived allocations (ids and UTF-8 strings).
//!
//! Limits are expressed in UTF-8 bytes for strings (`str::len()`).

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;

use db::MAX_PAGE_SIZE;

/// Maximum number of `i64` ids accepted per repeated query parameter (`?model_ids=1&model_ids=2`).
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

const fn validate_id_list(field: &'static str, ids: &[i64]) -> Result<(), QueryBoundsError> {
    if ids.len() > MAX_ID_LIST_ITEMS {
        return Err(QueryBoundsError::TooManyIds {
            field,
            max: MAX_ID_LIST_ITEMS,
        });
    }

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

/// Full query bounds for the paginated model and group list endpoints and their share
/// variants. Only the group endpoints take a raw `model_ids_str`; model endpoints pass `None`.
pub fn validate_list_query_bounds(
    query_bounds: PaginatedListQueryBounds<'_>,
    model_ids_str: Option<&str>,
) -> Result<(), QueryBoundsError> {
    validate_id_list("model_ids", query_bounds.model_ids)?;
    validate_id_list("group_ids", query_bounds.group_ids)?;
    validate_id_list("label_ids", query_bounds.label_ids)?;
    validate_optional_str(
        query_bounds.text_search,
        "text_search",
        MAX_QUERY_TEXT_BYTES,
    )?;
    validate_optional_str(query_bounds.order_by, "order_by", MAX_ORDER_BY_BYTES)?;
    validate_optional_str(model_ids_str, "model_ids_str", MAX_MODEL_IDS_STR_BYTES)?;
    validate_pagination(query_bounds.page, query_bounds.page_size)?;

    Ok(())
}

/// HTTP 400 response for [`QueryBoundsError`] (for handlers that `return Ok(...)`).
#[must_use]
pub fn bad_request(e: &QueryBoundsError) -> Response {
    (StatusCode::BAD_REQUEST, e.to_string()).into_response()
}

const fn validate_optional_str(
    value: Option<&str>,
    field: &'static str,
    max_bytes: usize,
) -> Result<(), QueryBoundsError> {
    if let Some(value) = value
        && value.len() > max_bytes
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
pub fn parse_comma_separated_i64(text: &str, max: usize) -> Result<Vec<i64>, QueryBoundsError> {
    let mut ids = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Ok(id) = part.parse::<i64>() {
            if ids.len() >= max {
                return Err(QueryBoundsError::TooManyCommaSeparatedIds { max });
            }
            ids.push(id);
        }
    }

    Ok(ids)
}

/// Converts a request-parameter vector into an option, treating an empty list as absent.
///
/// Never pass security-scoped ids (e.g. a share's model ids) through this: an
/// empty list becomes `None`, which the db layer reads as "no restriction" and
/// answers with every model the user owns. Pass them as `Some(ids)` instead.
#[must_use]
pub fn none_if_empty<T>(ids: Vec<T>) -> Option<Vec<T>> {
    if ids.is_empty() { None } else { Some(ids) }
}

pub fn optional_comma_separated_model_ids(
    model_ids_str: Option<&str>,
) -> Result<Option<Vec<i64>>, QueryBoundsError> {
    model_ids_str.map_or(Ok(None), |model_ids_str| {
        parse_comma_separated_i64(model_ids_str, MAX_ID_LIST_ITEMS).map(Some)
    })
}

/// Parses an `order_by` value; oversized or unknown values fall back to `default`.
#[must_use]
pub fn parse_order_by_bounded<T: std::str::FromStr>(order_by: &str, default: T) -> T {
    if order_by.len() > MAX_ORDER_BY_BYTES {
        return default;
    }

    T::from_str(order_by).unwrap_or(default)
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
    use db::{MAX_PAGE_SIZE, model_db::ModelOrderBy};

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
    fn validate_list_query_bounds_matches_individual_validators() {
        let ok = PaginatedListQueryBounds {
            model_ids: &[],
            group_ids: &[],
            label_ids: &[],
            text_search: None,
            order_by: None,
            page: 1,
            page_size: 50,
        };
        assert!(validate_list_query_bounds(ok, None).is_ok());
        assert_eq!(
            validate_list_query_bounds(PaginatedListQueryBounds { page: 0, ..ok }, None),
            Err(QueryBoundsError::PageOutOfRange { max: MAX_PAGE })
        );
    }

    #[test]
    fn validate_list_query_bounds_includes_model_ids_str() {
        let base = PaginatedListQueryBounds {
            model_ids: &[],
            group_ids: &[],
            label_ids: &[],
            text_search: None,
            order_by: None,
            page: 1,
            page_size: 50,
        };
        assert!(validate_list_query_bounds(base, None).is_ok());
        let long = "a".repeat(MAX_MODEL_IDS_STR_BYTES + 1);
        assert_eq!(
            validate_list_query_bounds(base, Some(&long)),
            Err(QueryBoundsError::StringTooLong {
                field: "model_ids_str",
                max: MAX_MODEL_IDS_STR_BYTES,
            })
        );
    }

    #[test]
    fn none_if_empty_maps_empty_to_none_and_keeps_values() {
        assert_eq!(none_if_empty(Vec::<i64>::new()), None);
        assert_eq!(none_if_empty(vec![1, 2]), Some(vec![1, 2]));
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
    fn parse_order_by_bounded_model_accepts_known_variant() {
        assert_eq!(
            parse_order_by_bounded("NameAsc", ModelOrderBy::AddedDesc),
            ModelOrderBy::NameAsc
        );
    }

    #[test]
    fn parse_order_by_bounded_skips_parsing_above_byte_limit() {
        #[derive(Debug, PartialEq)]
        struct ParsedLength(usize);

        impl std::str::FromStr for ParsedLength {
            type Err = std::convert::Infallible;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                assert!(
                    value.len() <= MAX_ORDER_BY_BYTES,
                    "oversized input was parsed"
                );

                Ok(Self(value.len()))
            }
        }

        assert_eq!(
            parse_order_by_bounded(&"a".repeat(MAX_ORDER_BY_BYTES), ParsedLength(0)),
            ParsedLength(MAX_ORDER_BY_BYTES)
        );
        for oversized in [
            "a".repeat(MAX_ORDER_BY_BYTES + 1),
            "é".repeat(MAX_ORDER_BY_BYTES / 2 + 1),
        ] {
            assert_eq!(
                parse_order_by_bounded(&oversized, ParsedLength(0)),
                ParsedLength(0)
            );
        }
    }

    /// Handlers return this response verbatim, so the status and text are client-visible.
    async fn rejection_body(result: Result<(), QueryBoundsError>) -> String {
        let response = bad_request(&result.expect_err("query bounds should be rejected"));
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("rejection body should be readable");

        String::from_utf8(body.to_vec()).expect("rejection body should be UTF-8")
    }

    #[tokio::test]
    async fn list_query_bounds_accept_limits_and_reject_one_past_each() {
        let ids_at_limit = vec![0_i64; MAX_ID_LIST_ITEMS];
        let ids_over_limit = vec![0_i64; MAX_ID_LIST_ITEMS + 1];
        let text_at_limit = "a".repeat(MAX_QUERY_TEXT_BYTES);
        let text_over_limit = "a".repeat(MAX_QUERY_TEXT_BYTES + 1);
        let multibyte_text_over_limit = "é".repeat(MAX_QUERY_TEXT_BYTES / 2 + 1);
        let order_by_at_limit = "a".repeat(MAX_ORDER_BY_BYTES);
        let order_by_over_limit = "a".repeat(MAX_ORDER_BY_BYTES + 1);
        let model_ids_str_at_limit = "1".repeat(MAX_MODEL_IDS_STR_BYTES);
        let model_ids_str_over_limit = "1".repeat(MAX_MODEL_IDS_STR_BYTES + 1);
        let at_limit = PaginatedListQueryBounds {
            model_ids: &ids_at_limit,
            group_ids: &ids_at_limit,
            label_ids: &ids_at_limit,
            text_search: Some(&text_at_limit),
            order_by: Some(&order_by_at_limit),
            page: MAX_PAGE,
            page_size: MAX_PAGE_SIZE,
        };
        let cases = [
            (
                PaginatedListQueryBounds {
                    model_ids: &ids_over_limit,
                    ..at_limit
                },
                "Too many values in model_ids; maximum allowed is 10000",
            ),
            (
                PaginatedListQueryBounds {
                    group_ids: &ids_over_limit,
                    ..at_limit
                },
                "Too many values in group_ids; maximum allowed is 10000",
            ),
            (
                PaginatedListQueryBounds {
                    label_ids: &ids_over_limit,
                    ..at_limit
                },
                "Too many values in label_ids; maximum allowed is 10000",
            ),
            (
                PaginatedListQueryBounds {
                    text_search: Some(&text_over_limit),
                    ..at_limit
                },
                "text_search exceeds maximum length of 8192 bytes (UTF-8)",
            ),
            (
                PaginatedListQueryBounds {
                    text_search: Some(&multibyte_text_over_limit),
                    ..at_limit
                },
                "text_search exceeds maximum length of 8192 bytes (UTF-8)",
            ),
            (
                PaginatedListQueryBounds {
                    order_by: Some(&order_by_over_limit),
                    ..at_limit
                },
                "order_by exceeds maximum length of 256 bytes (UTF-8)",
            ),
        ];

        assert_eq!(validate_list_query_bounds(at_limit, None), Ok(()));
        assert_eq!(
            validate_list_query_bounds(at_limit, Some(&model_ids_str_at_limit)),
            Ok(())
        );
        for (bounds, expected_body) in cases {
            assert_eq!(
                rejection_body(validate_list_query_bounds(bounds, None)).await,
                expected_body
            );
            assert_eq!(
                rejection_body(validate_list_query_bounds(
                    bounds,
                    Some(&model_ids_str_at_limit)
                ))
                .await,
                expected_body
            );
        }
        assert_eq!(
            rejection_body(validate_list_query_bounds(
                at_limit,
                Some(&model_ids_str_over_limit)
            ))
            .await,
            "model_ids_str exceeds maximum length of 262144 bytes (UTF-8)"
        );
    }

    #[tokio::test]
    async fn list_query_bounds_reject_out_of_range_pagination() {
        let first_page = PaginatedListQueryBounds {
            model_ids: &[],
            group_ids: &[],
            label_ids: &[],
            text_search: None,
            order_by: None,
            page: 1,
            page_size: 1,
        };
        let cases = [
            (0, 1, "page must be between 1 and 1000000 inclusive"),
            (
                MAX_PAGE + 1,
                1,
                "page must be between 1 and 1000000 inclusive",
            ),
            (1, 0, "page_size must be between 1 and 1000 inclusive"),
            (
                1,
                MAX_PAGE_SIZE + 1,
                "page_size must be between 1 and 1000 inclusive",
            ),
        ];

        assert_eq!(validate_list_query_bounds(first_page, None), Ok(()));
        for (page, page_size, expected_body) in cases {
            let bounds = PaginatedListQueryBounds {
                page,
                page_size,
                ..first_page
            };
            assert_eq!(
                rejection_body(validate_list_query_bounds(bounds, None)).await,
                expected_body
            );
        }
    }

    #[tokio::test]
    async fn list_query_bounds_report_model_ids_first_when_everything_is_invalid() {
        let ids_over_limit = vec![0_i64; MAX_ID_LIST_ITEMS + 1];
        let text_over_limit = "a".repeat(MAX_QUERY_TEXT_BYTES + 1);
        let model_ids_str_over_limit = "1".repeat(MAX_MODEL_IDS_STR_BYTES + 1);
        let all_invalid = PaginatedListQueryBounds {
            model_ids: &ids_over_limit,
            group_ids: &ids_over_limit,
            label_ids: &ids_over_limit,
            text_search: Some(&text_over_limit),
            order_by: Some(&text_over_limit),
            page: 0,
            page_size: 0,
        };

        assert_eq!(
            rejection_body(validate_list_query_bounds(all_invalid, None)).await,
            "Too many values in model_ids; maximum allowed is 10000"
        );
        assert_eq!(
            rejection_body(validate_list_query_bounds(
                all_invalid,
                Some(&model_ids_str_over_limit)
            ))
            .await,
            "Too many values in model_ids; maximum allowed is 10000"
        );
    }

    #[tokio::test]
    async fn optional_comma_separated_model_ids_parses_and_bounds_model_ids_str() {
        let ids_at_limit = vec!["5"; MAX_ID_LIST_ITEMS].join(",");
        let ids_at_limit_with_garbage = format!("{ids_at_limit},x,,1.5");
        let ids_over_limit = format!("{ids_at_limit},5");

        assert_eq!(optional_comma_separated_model_ids(None), Ok(None));
        // Pins the deferred bug: an empty value matches no groups rather than disabling the filter.
        assert_eq!(
            optional_comma_separated_model_ids(Some("")),
            Ok(Some(vec![]))
        );
        assert_eq!(
            optional_comma_separated_model_ids(Some("42")),
            Ok(Some(vec![42]))
        );
        assert_eq!(
            optional_comma_separated_model_ids(Some(" 1, 2 ,3,3,-7,+8 ")),
            Ok(Some(vec![1, 2, 3, 3, -7, 8]))
        );
        assert_eq!(
            optional_comma_separated_model_ids(Some("abc,1.5,0x10,,9223372036854775808")),
            Ok(Some(vec![]))
        );
        assert_eq!(
            optional_comma_separated_model_ids(Some("1,abc,2")),
            Ok(Some(vec![1, 2]))
        );
        assert_eq!(
            optional_comma_separated_model_ids(Some(&ids_at_limit_with_garbage))
                .map(|ids| ids.map(|ids| ids.len())),
            Ok(Some(MAX_ID_LIST_ITEMS))
        );
        assert_eq!(
            rejection_body(optional_comma_separated_model_ids(Some(&ids_over_limit)).map(|_| ()))
                .await,
            "Too many model_ids in model_ids_str; maximum allowed is 10000"
        );
    }

    #[test]
    fn parse_order_by_bounded_maps_every_variant_and_falls_back_on_anything_else() {
        let model_variants = [
            ("AddedAsc", ModelOrderBy::AddedAsc),
            ("AddedDesc", ModelOrderBy::AddedDesc),
            ("NameAsc", ModelOrderBy::NameAsc),
            ("NameDesc", ModelOrderBy::NameDesc),
            ("SizeAsc", ModelOrderBy::SizeAsc),
            ("SizeDesc", ModelOrderBy::SizeDesc),
            ("ModifiedAsc", ModelOrderBy::ModifiedAsc),
            ("ModifiedDesc", ModelOrderBy::ModifiedDesc),
        ];
        let group_variants = [
            ("CreatedAsc", db::group_db::GroupOrderBy::CreatedAsc),
            ("CreatedDesc", db::group_db::GroupOrderBy::CreatedDesc),
            ("NameAsc", db::group_db::GroupOrderBy::NameAsc),
            ("NameDesc", db::group_db::GroupOrderBy::NameDesc),
            ("ModifiedAsc", db::group_db::GroupOrderBy::ModifiedAsc),
            ("ModifiedDesc", db::group_db::GroupOrderBy::ModifiedDesc),
        ];
        let unknown_at_limit = "a".repeat(MAX_ORDER_BY_BYTES);
        let invalid = [
            "",
            "nameasc",
            "NAMEASC",
            " NameAsc",
            "NameAsc ",
            "name_asc",
            "AddedAsc,NameAsc",
            unknown_at_limit.as_str(),
        ];

        // Two different defaults so a parsed value can never coincide with the fallback.
        for (name, expected) in model_variants {
            assert_eq!(
                parse_order_by_bounded(name, ModelOrderBy::AddedAsc),
                expected
            );
            assert_eq!(
                parse_order_by_bounded(name, ModelOrderBy::AddedDesc),
                expected
            );
        }
        for (name, expected) in group_variants {
            assert_eq!(
                parse_order_by_bounded(name, db::group_db::GroupOrderBy::CreatedAsc),
                expected
            );
            assert_eq!(
                parse_order_by_bounded(name, db::group_db::GroupOrderBy::CreatedDesc),
                expected
            );
        }
        for order_by in invalid {
            assert_eq!(
                parse_order_by_bounded(order_by, ModelOrderBy::SizeDesc),
                ModelOrderBy::SizeDesc
            );
            assert_eq!(
                parse_order_by_bounded(order_by, db::group_db::GroupOrderBy::ModifiedAsc),
                db::group_db::GroupOrderBy::ModifiedAsc
            );
        }
    }
}
