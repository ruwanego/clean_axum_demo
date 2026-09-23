//! Conditional requests (RFC 9110): `ETag` with `If-None-Match` and `If-Match`.
//!
//! An entity tag is derived from the row's id and `modified_at`, so it changes
//! whenever the row is updated. Two uses:
//!
//! - `GET` returns `ETag`; a client resending it as `If-None-Match` gets `304 Not Modified`.
//! - `PUT` with `If-Match` only applies when the tag still matches, which stops a client
//!   from overwriting an edit it never saw (the lost update problem). Requests without
//!   `If-Match` are still accepted, so this is opt-in per client.

use axum::http::{HeaderMap, header};
use chrono::{DateTime, Utc};

use super::error::AppError;

/// Builds the entity tag for a resource version.
/// `modified_at` is absent only for rows written before the column existed.
pub fn entity_tag(id: &str, modified_at: Option<DateTime<Utc>>) -> String {
    let version = modified_at
        .map(|t| t.timestamp_micros())
        .unwrap_or_default();
    format!("\"{id}-{version}\"")
}

/// Returns true when `If-None-Match` names the current tag, i.e. the client's copy
/// is still fresh and the handler should answer `304 Not Modified`.
pub fn is_not_modified(headers: &HeaderMap, etag: &str) -> bool {
    header_values(headers, header::IF_NONE_MATCH).is_some_and(|value| matches_tag(value, etag))
}

/// Enforces `If-Match` when the client sent it.
/// Returns `PreconditionFailed` (412) if the resource changed in the meantime.
/// A request without the header is allowed through.
pub fn check_if_match(headers: &HeaderMap, etag: &str) -> Result<(), AppError> {
    match header_values(headers, header::IF_MATCH) {
        Some(value) if !matches_tag(value, etag) => Err(AppError::PreconditionFailed),
        _ => Ok(()),
    }
}

fn header_values(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// Matches a comma-separated list of tags against the current one, honouring `*`
/// and treating weak tags (`W/"..."`) as equal to their strong form.
fn matches_tag(header_value: &str, etag: &str) -> bool {
    header_value
        .split(',')
        .map(str::trim)
        .any(|candidate| candidate == "*" || strip_weak(candidate) == strip_weak(etag))
}

fn strip_weak(tag: &str) -> &str {
    tag.strip_prefix("W/").unwrap_or(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAG: &str = "\"abc-123\"";

    fn headers(name: header::HeaderName, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(name, value.parse().unwrap());
        headers
    }

    #[test]
    fn tag_changes_with_modified_at() {
        let now = Utc::now();
        assert_ne!(entity_tag("a", Some(now)), entity_tag("b", Some(now)));
        assert_ne!(
            entity_tag("a", Some(now)),
            entity_tag("a", Some(now + chrono::Duration::seconds(1)))
        );
    }

    #[test]
    fn if_none_match_detects_a_fresh_copy() {
        assert!(is_not_modified(&headers(header::IF_NONE_MATCH, TAG), TAG));
        assert!(is_not_modified(
            &headers(header::IF_NONE_MATCH, &format!("W/{TAG}")),
            TAG
        ));
        assert!(is_not_modified(&headers(header::IF_NONE_MATCH, "*"), TAG));
        assert!(!is_not_modified(
            &headers(header::IF_NONE_MATCH, "\"other\""),
            TAG
        ));
        assert!(!is_not_modified(&HeaderMap::new(), TAG));
    }

    #[test]
    fn if_match_guards_updates() {
        assert!(check_if_match(&headers(header::IF_MATCH, TAG), TAG).is_ok());
        assert!(check_if_match(&headers(header::IF_MATCH, "\"stale\", \"other\""), TAG).is_err());
        // Absent header: the update proceeds.
        assert!(check_if_match(&HeaderMap::new(), TAG).is_ok());
    }
}
