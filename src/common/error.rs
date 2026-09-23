use axum::{
    BoxError,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use sqlx::Error as SqlxError;
use thiserror::Error;
use tracing::error;

use super::problem::ProblemDetails;

/// AppError is an enum that represents various types of errors that can occur in the application.
/// It implements the `std::error::Error` trait and the `axum::response::IntoResponse` trait.
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] SqlxError), // Used for database-related errors

    #[error("Not found: {0}")]
    NotFound(String), // Used for not found errors

    #[error("Internal server error")]
    InternalError,

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Forbidden Request")]
    Forbidden,

    /// Used for file-related errors
    #[error("File data is empty")]
    InvalidFileData,

    #[error("File size exceeded")]
    FileSizeExceeded,

    #[error("Invalid file name")]
    InvalidFileName,

    #[error("Unsupported file extension")]
    UnsupportedFileExtension,

    /// Used for authentication-related errors
    #[error("Wrong credentials")]
    WrongCredentials,
    #[error("Missing credentials")]
    MissingCredentials,
    #[error("Invalid token")]
    InvalidToken,
    #[error("Token creation error")]
    TokenCreation,
    #[error("User not found")]
    UserNotFound,

    /// Optimistic concurrency: the client's If-Match did not match the current ETag.
    #[error("Precondition failed: the resource has changed")]
    PreconditionFailed,
}

impl AppError {
    /// Maps the error to its HTTP status, problem `type` slug and title.
    fn problem_kind(&self) -> (StatusCode, &'static str, &'static str) {
        match self {
            AppError::ValidationError(_) => (
                StatusCode::BAD_REQUEST,
                "validation-error",
                "Validation error",
            ),
            AppError::DatabaseError(_) | AppError::InternalError => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal-error",
                "Internal server error",
            ),
            AppError::NotFound(_) | AppError::UserNotFound => {
                (StatusCode::NOT_FOUND, "not-found", "Resource not found")
            }
            AppError::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "Forbidden"),
            AppError::InvalidFileData
            | AppError::FileSizeExceeded
            | AppError::InvalidFileName
            | AppError::UnsupportedFileExtension => {
                (StatusCode::BAD_REQUEST, "invalid-file", "Invalid file")
            }
            AppError::WrongCredentials | AppError::InvalidToken => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication failed",
            ),
            AppError::MissingCredentials => (
                StatusCode::BAD_REQUEST,
                "missing-credentials",
                "Missing credentials",
            ),
            AppError::TokenCreation => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal-error",
                "Internal server error",
            ),
            AppError::PreconditionFailed => (
                StatusCode::PRECONDITION_FAILED,
                "precondition-failed",
                "Precondition failed",
            ),
        }
    }

    /// The client-facing explanation. Internal causes are deliberately withheld:
    /// database and other internal errors are logged instead of returned.
    fn detail(&self) -> String {
        match self {
            AppError::DatabaseError(err) => {
                error!("Database error: {err}");
                "The request could not be completed".to_string()
            }
            AppError::InternalError | AppError::TokenCreation => {
                error!("Internal error: {self}");
                "The request could not be completed".to_string()
            }
            other => other.to_string(),
        }
    }

    /// Renders this error as an RFC 9457 problem details document.
    pub fn to_problem(&self) -> ProblemDetails {
        let (status, kind, title) = self.problem_kind();
        ProblemDetails::new(status, kind, title).with_detail(self.detail())
    }
}

/// Converts the AppError into an RFC 9457 `application/problem+json` response.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        self.to_problem().into_response()
    }
}

/// handle_error is a function that middlewares the error handling in the application.
/// It takes a BoxError as input and returns an HTTP response.
/// It maps the error to an appropriate HTTP status code and constructs a problem details body.
/// The function is used to handle errors that occur during the request processing.
/// It is designed to be used with the axum framework.
pub async fn handle_error(error: BoxError) -> impl IntoResponse {
    let (status, kind, title) = if error.is::<tower::timeout::error::Elapsed>() {
        (StatusCode::REQUEST_TIMEOUT, "timeout", "Request timeout")
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal-error",
            "Internal server error",
        )
    };

    error!(?status, message = %error, "Request failed");

    ProblemDetails::new(status, kind, title).with_detail(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_errors_are_not_exposed_to_clients() {
        let problem = AppError::DatabaseError(SqlxError::RowNotFound).to_problem();

        assert_eq!(problem.status, 500);
        assert_eq!(problem.type_uri, "/problems/internal-error");
        assert_eq!(
            problem.detail.as_deref(),
            Some("The request could not be completed")
        );
    }

    #[test]
    fn validation_errors_keep_their_message() {
        let problem = AppError::ValidationError("email is invalid".into()).to_problem();

        assert_eq!(problem.status, 400);
        assert_eq!(
            problem.detail.as_deref(),
            Some("Validation error: email is invalid")
        );
    }
}
