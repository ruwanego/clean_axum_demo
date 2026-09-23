use crate::common::dto::RestApiResponse;
use crate::common::etag::{check_if_match, entity_tag, is_not_modified};
use crate::common::pagination::{Page, PageQuery};
use crate::common::{app_state::AppState, error::AppError, jwt::Claims};

use crate::domains::device::dto::device_dto::{
    CreateDeviceDto, DeviceDto, UpdateDeviceDto, UpdateManyDevicesDto,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};

/// This function creates a router for getting a device by ID
/// It will return a device if found, otherwise it will return an error
#[utoipa::path(
    get,
    path = "/device/{id}",
    responses((status = 200, description = "Get device by ID", body = DeviceDto)),
    tag = "Devices"
)]
pub async fn get_device_by_id(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let device = state.device_service.get_device_by_id(id).await?;
    let etag = entity_tag(&device.id, device.modified_at);

    // The client already has this version.
    if is_not_modified(&headers, &etag) {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response());
    }

    Ok(([(header::ETAG, etag)], RestApiResponse::success(device)).into_response())
}

/// This function creates a router for getting all devices
/// It will return a list of devices
#[utoipa::path(
    get,
    path = "/device",
    params(PageQuery),
    responses((status = 200, description = "List devices (cursor paginated)", body = Page<DeviceDto>)),
    tag = "Devices"
)]
pub async fn get_devices(
    State(state): State<AppState>,
    Query(page): Query<PageQuery>,
) -> Result<impl IntoResponse, AppError> {
    let devices = state.device_service.get_devices(page).await?;
    Ok(RestApiResponse::success(devices))
}

/// This function creates a router for creating a new device
/// It will create a new device in the database
/// It will return the created device
#[utoipa::path(
    post,
    path = "/device",
    request_body = CreateDeviceDto,
    responses((status = 200, description = "Create a new device", body = DeviceDto)),
    tag = "Devices"
)]
pub async fn create_device(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<CreateDeviceDto>,
) -> Result<impl IntoResponse, AppError> {
    // Set the modified_by field to the current user's ID.
    let mut payload = payload;
    payload.modified_by = claims.sub.clone().to_string();

    let device = state.device_service.create_device(payload).await?;
    Ok(RestApiResponse::success(device))
}

/// This function creates a router for updating a device
/// It will update the device in the database
/// It will return the updated device
#[utoipa::path(
    put,
    path = "/device/{id}",
    request_body = UpdateDeviceDto,
    responses((status = 200, description = "Update device", body = DeviceDto)),
    tag = "Devices"
)]
pub async fn update_device(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(payload): Json<UpdateDeviceDto>,
) -> Result<Response, AppError> {
    // Optimistic concurrency: reject the write if the caller's copy is stale.
    let current = state.device_service.get_device_by_id(id.clone()).await?;
    check_if_match(&headers, &entity_tag(&current.id, current.modified_at))?;

    // Set the modified_by field to the current user's ID.
    let mut payload = payload;
    payload.modified_by = claims.sub.clone().to_string();

    let device = state.device_service.update_device(id, payload).await?;
    let etag = entity_tag(&device.id, device.modified_at);

    Ok(([(header::ETAG, etag)], RestApiResponse::success(device)).into_response())
}

/// This function creates a router for deleting a device
/// It will delete the device from the database
/// It will return a message indicating the result of the operation
#[utoipa::path(
    delete,
    path = "/device/{id}",
    responses((status = 200, description = "Device deleted")),
    tag = "Devices"
)]
pub async fn delete_device(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let message = state.device_service.delete_device(id).await?;

    Ok(RestApiResponse::success_with_message(message, ()))
}

/// This function creates a router for batch updating devices
/// It will update multiple devices in the database
/// It will return a message indicating the result of the operation
#[utoipa::path(
    put,
    path = "/device/batch/{user_id}",
    request_body = UpdateManyDevicesDto,
    responses((status = 200, description = "Batch update devices")),
    tag = "Devices"
)]
pub async fn update_many_devices(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(user_id): Path<String>,
    Json(payload): Json<UpdateManyDevicesDto>,
) -> Result<impl IntoResponse, AppError> {
    let modified_by = claims.sub.clone().to_string();

    let message = state
        .device_service
        .update_many_devices(user_id, modified_by, payload)
        .await?;

    Ok(RestApiResponse::success_with_message(message, ()))
}
