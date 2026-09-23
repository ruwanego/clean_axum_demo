//! RFC 9457 "Problem Details for HTTP APIs" error responses.
//!
//! Every error leaves the API as `application/problem+json`:
//!
//! ```json
//! { "type": "/problems/validation-error", "title": "Validation error",
//!   "status": 400, "detail": "Invalid input: email is not valid" }
//! ```
//!
//! `detail` is always a message meant for clients. Internal causes (database errors,
//! storage failures) are logged instead, never echoed back.

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Media type required by RFC 9457.
pub const PROBLEM_JSON: &str = "application/problem+json";

/// A problem details document (RFC 9457).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProblemDetails {
    /// URI reference identifying the problem type.
    #[serde(rename = "type")]
    pub type_uri: String,
    /// Short, human-readable summary of the problem type.
    pub title: String,
    /// HTTP status code.
    pub status: u16,
    /// Human-readable explanation specific to this occurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// URI reference identifying the specific occurrence (the request path).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
}

impl ProblemDetails {
    /// Builds a problem document. `kind` becomes the `type` URI, e.g. `not-found`.
    pub fn new(status: StatusCode, kind: &str, title: impl Into<String>) -> Self {
        Self {
            type_uri: format!("/problems/{kind}"),
            title: title.into(),
            status: status.as_u16(),
            detail: None,
            instance: None,
        }
    }

    /// Adds the client-facing explanation.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Records the request path this problem occurred on.
    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    fn status_code(&self) -> StatusCode {
        StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl IntoResponse for ProblemDetails {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let mut response = (status, Json(self)).into_response();
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON));
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omits_absent_fields_and_renames_type() {
        let json = serde_json::to_string(&ProblemDetails::new(
            StatusCode::NOT_FOUND,
            "not-found",
            "Not found",
        ))
        .unwrap();

        assert_eq!(
            json,
            r#"{"type":"/problems/not-found","title":"Not found","status":404}"#
        );
    }

    #[test]
    fn response_uses_problem_json_content_type() {
        let response = ProblemDetails::new(StatusCode::BAD_REQUEST, "validation-error", "Bad")
            .with_detail("nope")
            .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[CONTENT_TYPE], PROBLEM_JSON);
    }
}
