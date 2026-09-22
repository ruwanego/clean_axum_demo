use crate::common::{app_state::AppState, dto::RestApiResponse, error::AppError};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

/// This function serves a protected file from storage.
/// It will return the file as a response with the appropriate content type and headers.
/// If the file is not found, it will return a 404 error.
#[utoipa::path(
    get,
    path = "/file/{file_id}",
    responses((status = 200, description = "Serve protected file")),
    tag = "Files"
)]
/// Serve a protected file from storage.
pub async fn serve_protected_file(
    State(state): State<AppState>,
    Path(file_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let file_metadata = state.file_service.get_file_metadata(file_id).await?;

    // If the file is not found, return a 404.
    let file_metadata = file_metadata.ok_or_else(|| AppError::NotFound("File not found".into()))?;

    // Stream the file from storage (local disk or S3).
    let stream = state
        .file_service
        .read_file(&file_metadata.file_relative_path)
        .await?;

    // Build a full response with content type header set to the file's MIME type.
    // Here we use file_metadata.content_type that should contain a valid MIME string.
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, file_metadata.content_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!(
                "attachment; filename=\"{}\"",
                sanitize_filename(&file_metadata.origin_file_name)
            ),
        )
        .body(Body::from_stream(stream))
        .map_err(|err| {
            tracing::error!("Error building response: {}", err);
            AppError::InternalError
        })?;

    Ok(response)
}

/// Serves a private asset by its storage key, e.g.
/// `GET /assets/private/profile_picture/<name>`. Requires authentication (see `app.rs`).
pub async fn serve_private_asset(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let stream = state.file_service.read_file(&key).await?;
    let content_type = mime_guess::from_path(&key).first_or_octet_stream();

    Ok((
        [(header::CONTENT_TYPE, content_type.to_string())],
        Body::from_stream(stream),
    ))
}

/// Keeps a user-supplied file name safe inside a quoted header value.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | '\\'))
        .collect()
}

/// This function deletes a file from the server's filesystem and database.
/// It will return a success message if the deletion is successful, or an error if not.
#[utoipa::path(
    delete,
    path = "/file/{file_id}",
    responses((status = 200, description = "Delete file")),
    tag = "Files"
)]
/// Delete a file from the server's filesystem and database.
pub async fn delete_file(
    State(state): State<AppState>,
    Path(file_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let message = state.file_service.delete_file(file_id).await?;
    Ok(RestApiResponse::success_with_message(message, ()))
}
