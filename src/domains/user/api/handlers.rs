use crate::{
    common::{
        app_state::AppState,
        dto::RestApiResponse,
        error::AppError,
        etag::{check_if_match, entity_tag, is_not_modified},
        jwt::Claims,
        multipart_helper::parse_multipart_to_maps,
        pagination::{Page, PageQuery},
    },
    domains::{
        file::dto::file_dto::UploadFileDto,
        user::dto::user_dto::{CreateUserMultipartDto, SearchUserDto, UpdateUserDto, UserDto},
    },
};

use axum::{
    Extension, Json,
    extract::{Multipart, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};

use validator::Validate;

#[utoipa::path(
    get,
    path = "/user/{id}",
    responses((status = 200, description = "Get user by ID", body = UserDto)),
    tag = "Users"
)]
pub async fn get_user_by_id(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let user = state.user_service.get_user_by_id(id).await?;
    let etag = entity_tag(&user.id, user.modified_at);

    // The client already has this version.
    if is_not_modified(&headers, &etag) {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response());
    }

    Ok(([(header::ETAG, etag)], RestApiResponse::success(user)).into_response())
}

#[utoipa::path(
    post,
    path = "/user/list",
    request_body = SearchUserDto,
    responses((status = 200, description = "List users by condition", body = [UserDto])),
    tag = "Users"
)]
pub async fn get_user_list(
    State(state): State<AppState>,
    Json(payload): Json<SearchUserDto>,
) -> Result<impl IntoResponse, AppError> {
    let users = state.user_service.get_user_list(payload).await?;
    Ok(RestApiResponse::success(users))
}

#[utoipa::path(
    get,
    path = "/user",
    params(PageQuery),
    responses((status = 200, description = "List users (cursor paginated)", body = Page<UserDto>)),
    tag = "Users"
)]
pub async fn get_users(
    State(state): State<AppState>,
    Query(page): Query<PageQuery>,
) -> Result<impl IntoResponse, AppError> {
    let users = state.user_service.get_users(page).await?;
    Ok(RestApiResponse::success(users))
}

#[utoipa::path(
    post,
    path = "/user",
    request_body(
        content = CreateUserMultipartDto,
        content_type = "multipart/form-data",
        description = "User creation with optional profile picture upload"
    ),
    responses((status = 200, description = "Create a new user", body = UserDto)),
    tag = "Users"
)]
pub async fn create_user(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let modified_by = claims.sub.clone().to_string();

    let (mut fields, mut files) =
        parse_multipart_to_maps(multipart, &state.config.asset_allowed_extensions_pattern).await?;

    // Validate required fields.
    let username = fields
        .remove("username")
        .ok_or(AppError::ValidationError("Missing username".into()))?;
    let email = fields
        .remove("email")
        .ok_or(AppError::ValidationError("Missing email".into()))?;

    // Prepare the CreateUser DTO.
    let create_user = CreateUserMultipartDto {
        username,
        email,
        modified_by: modified_by.clone(),
        profile_picture: None,
    };

    // Validate the CreateUser DTO.
    create_user
        .validate()
        .map_err(|err| AppError::ValidationError(format!("Invalid input: {}", err)))?;

    let mut upload_file_dto = None;

    // If a profile picture was uploaded, handle it.
    if files.contains_key("profile_picture") {
        let profile_files = files
            .remove("profile_picture")
            .ok_or(AppError::ValidationError("Missing profile picture".into()))?;

        if let Some(file) = profile_files.into_iter().next() {
            upload_file_dto = Some(UploadFileDto {
                file,
                user_id: None,
                modified_by,
            });
        }
    }

    let user = state
        .user_service
        .create_user(create_user, upload_file_dto.as_mut())
        .await?;

    Ok(RestApiResponse::success(user))
}

#[utoipa::path(
    put,
    path = "/user/{id}",
    request_body = UpdateUserDto,
    responses((status = 200, description = "Update user", body = UserDto)),
    tag = "Users"
)]
pub async fn update_user(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(payload): Json<UpdateUserDto>,
) -> Result<Response, AppError> {
    // Optimistic concurrency: reject the write if the caller's copy is stale.
    let current = state.user_service.get_user_by_id(id.clone()).await?;
    check_if_match(&headers, &entity_tag(&current.id, current.modified_at))?;

    payload.validate().map_err(|err| {
        tracing::error!("Validation error: {err}");
        AppError::ValidationError(format!("Invalid input: {}", err))
    })?;

    // Set the modified_by field to the current user's ID.
    let mut payload = payload;
    payload.modified_by = claims.sub.clone().to_string();

    let user = state.user_service.update_user(id, payload).await?;
    let etag = entity_tag(&user.id, user.modified_at);

    Ok(([(header::ETAG, etag)], RestApiResponse::success(user)).into_response())
}

#[utoipa::path(
    delete,
    path = "/user/{id}",
    responses((status = 200, description = "User deleted")),
    tag = "Users"
)]
pub async fn delete_user(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let message = state.user_service.delete_user(id).await?;
    Ok(RestApiResponse::success_with_message(message, ()))
}
