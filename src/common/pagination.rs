//! Cursor (keyset) pagination.
//!
//! Offset pagination skips rows by position, so inserts and deletes between requests
//! shift the window and rows get repeated or missed. A cursor instead points at the
//! last row returned — `(created_at, id)` here — and the next query continues strictly
//! after it, which stays correct while the table changes underneath.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::error::AppError;

/// Default number of items per page when the client does not ask for a size.
pub const DEFAULT_PAGE_SIZE: u32 = 50;
/// Upper bound on page size, so a client cannot ask for the whole table.
pub const MAX_PAGE_SIZE: u32 = 200;

/// Query parameters for a cursor-paginated list endpoint: `?limit=50&cursor=...`.
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
pub struct PageQuery {
    /// Maximum number of items to return (default 50, max 200).
    pub limit: Option<u32>,
    /// Opaque cursor from the previous page's `next_cursor`.
    pub cursor: Option<String>,
}

impl PageQuery {
    /// Page size, clamped to `1..=MAX_PAGE_SIZE`.
    pub fn limit(&self) -> u32 {
        self.limit
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE)
    }

    /// Decodes the cursor, if the client sent one.
    pub fn cursor(&self) -> Result<Option<Cursor>, AppError> {
        self.cursor.as_deref().map(Cursor::decode).transpose()
    }
}

/// Position of the last item of a page, ordered by `(created_at DESC, id DESC)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub created_at: DateTime<Utc>,
    pub id: String,
}

impl Cursor {
    pub fn new(created_at: DateTime<Utc>, id: impl Into<String>) -> Self {
        Self {
            created_at,
            id: id.into(),
        }
    }

    /// Encodes the cursor as base64url. Opaque by contract: clients must not parse it.
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(format!("{}|{}", self.created_at.to_rfc3339(), self.id))
    }

    /// Decodes a cursor produced by [`Cursor::encode`].
    pub fn decode(raw: &str) -> Result<Self, AppError> {
        let invalid = || AppError::ValidationError("Invalid cursor".into());

        let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
        let decoded = String::from_utf8(bytes).map_err(|_| invalid())?;
        let (timestamp, id) = decoded.split_once('|').ok_or_else(invalid)?;
        if id.is_empty() {
            return Err(invalid());
        }

        Ok(Self {
            created_at: DateTime::parse_from_rfc3339(timestamp)
                .map_err(|_| invalid())?
                .with_timezone(&Utc),
            id: id.to_string(),
        })
    }
}

/// One page of results plus the cursor for the next one.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Page<T> {
    /// The items in this page.
    pub items: Vec<T>,
    /// Cursor to pass as `?cursor=` for the next page; absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Whether another page exists.
    pub has_more: bool,
}

impl<T> Page<T> {
    /// Builds a page from `items`, which the caller fetched with `limit + 1` rows so
    /// the extra row reveals whether more data follows. `cursor_of` reads the cursor
    /// position from the last returned item.
    pub fn build(
        mut items: Vec<T>,
        limit: u32,
        cursor_of: impl Fn(&T) -> Option<Cursor>,
    ) -> Result<Self, AppError> {
        let has_more = items.len() > limit as usize;
        items.truncate(limit as usize);

        let next_cursor = if has_more {
            let last = items
                .last()
                .ok_or_else(|| AppError::ValidationError("Invalid pagination state".to_string()))?;
            cursor_of(last).map(|c| c.encode())
        } else {
            None
        };

        Ok(Self {
            items,
            next_cursor,
            has_more,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor() -> Cursor {
        Cursor::new(
            DateTime::parse_from_rfc3339("2026-09-22T10:20:30Z")
                .unwrap()
                .with_timezone(&Utc),
            "user-1",
        )
    }

    #[test]
    fn cursor_roundtrips() {
        assert_eq!(Cursor::decode(&cursor().encode()).unwrap(), cursor());
    }

    #[test]
    fn rejects_malformed_cursors() {
        for raw in ["not-base64!!", "", &URL_SAFE_NO_PAD.encode("no-separator")] {
            assert!(Cursor::decode(raw).is_err(), "{raw:?} should be rejected");
        }
    }

    #[test]
    fn limit_is_clamped() {
        let query = |limit| PageQuery {
            limit: Some(limit),
            cursor: None,
        };

        assert_eq!(PageQuery::default().limit(), DEFAULT_PAGE_SIZE);
        assert_eq!(query(0).limit(), 1);
        assert_eq!(query(10_000).limit(), MAX_PAGE_SIZE);
    }

    #[test]
    fn page_uses_the_extra_row_to_detect_more() {
        let rows = vec!["a", "b", "c"];
        let page = Page::build(rows, 2, |_| Some(cursor())).unwrap();

        assert_eq!(page.items, vec!["a", "b"]);
        assert!(page.has_more);
        assert_eq!(page.next_cursor, Some(cursor().encode()));

        let page = Page::build(vec!["a"], 2, |_| Some(cursor())).unwrap();
        assert!(!page.has_more);
        assert!(page.next_cursor.is_none());
    }
}
