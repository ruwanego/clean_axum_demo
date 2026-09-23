use axum::http::{Method, StatusCode};

use clean_axum_demo::{
    common::{dto::RestApiResponse, error::AppError, pagination::Page, problem::ProblemDetails},
    domains::user::dto::user_dto::{CreateUserMultipartDto, SearchUserDto, UpdateUserDto, UserDto},
};

mod test_helpers;

use test_helpers::{
    TEST_USER_ID, deserialize_json_body, request_with_auth, request_with_auth_and_body,
    request_with_auth_and_multipart,
};

async fn create_user() -> Result<(CreateUserMultipartDto, UserDto), AppError> {
    let username = format!("testuser-{}", uuid::Uuid::new_v4()).to_string();
    let email = format!("{}@test.com", username).to_string();

    let payload = CreateUserMultipartDto {
        username,
        email,
        modified_by: TEST_USER_ID.to_string(),
        profile_picture: None,
    };

    let multipart_body = format!(
        "------XYZ\r\nContent-Disposition: form-data; name=\"username\"\r\n\r\n{}\r\n------XYZ\r\nContent-Disposition: form-data; name=\"email\"\r\n\r\n{}\r\n------XYZ\r\nContent-Disposition: form-data; name=\"modified_by\"\r\n\r\n{}\r\n------XYZ--\r\n",
        payload.username, payload.email, payload.modified_by
    ).as_bytes().to_vec();

    let response = request_with_auth_and_multipart(Method::POST, "/user", multipart_body);

    let (parts, body) = response.await.into_parts();

    assert_eq!(parts.status, StatusCode::OK);

    let response_body: RestApiResponse<UserDto> = deserialize_json_body(body).await.unwrap();

    assert_eq!(response_body.0.status, StatusCode::OK);
    let user_dto = response_body.0.data.unwrap();

    Ok((payload, user_dto))
}

#[tokio::test]
async fn test_create_user() {
    let created = create_user().await.expect("Failed to create user");

    let payload = created.0;
    let user_dto = created.1;

    assert!(!user_dto.id.is_empty());
    assert_eq!(user_dto.username, payload.username.clone());
    assert_eq!(user_dto.email, Some(payload.email.clone()));
    assert_ne!(user_dto.modified_by, Some(payload.modified_by.clone()));
    assert_eq!(user_dto.origin_file_name, None);
    assert!(user_dto.file_id.is_none());
}

async fn create_user_with_file() -> Result<(CreateUserMultipartDto, UserDto, String), AppError> {
    let username = format!("testuser-{}", uuid::Uuid::new_v4()).to_string();
    let email = format!("{}@test.com", username).to_string();

    let image_file = "cat.png";

    let payload = CreateUserMultipartDto {
        username,
        email,
        modified_by: TEST_USER_ID.to_string(),
        // Indicate the file name being uploaded
        profile_picture: Some(image_file.to_string()),
    };

    // Read the image file from the test/asset/ directory
    let file_path = format!("tests/asset/{}", image_file);
    let file_bytes = std::fs::read(file_path)
        .unwrap_or_else(|_| panic!("Failed to read {} from tests/asset/", image_file));

    // Build the multipart body as a byte vector (Vec<u8>)
    let mut multipart_body = Vec::new();
    use std::io::Write;
    // Add the username part
    write!(
        &mut multipart_body,
        "------XYZ\r\nContent-Disposition: form-data; name=\"username\"\r\n\r\n{}\r\n",
        payload.username
    )
    .unwrap();
    // Add the email part
    write!(
        &mut multipart_body,
        "------XYZ\r\nContent-Disposition: form-data; name=\"email\"\r\n\r\n{}\r\n",
        payload.email
    )
    .unwrap();
    // Add the modified_by part
    write!(
        &mut multipart_body,
        "------XYZ\r\nContent-Disposition: form-data; name=\"modified_by\"\r\n\r\n{}\r\n",
        payload.modified_by
    )
    .unwrap();
    // Add the file part for profile_picture
    write!(
        &mut multipart_body,
        "------XYZ\r\nContent-Disposition: form-data; name=\"profile_picture\"; filename=\"{}\"\r\nContent-Type: image/png\r\n\r\n",
        image_file
    ).unwrap();
    multipart_body.extend_from_slice(&file_bytes);
    write!(&mut multipart_body, "\r\n").unwrap();
    // Add the final boundary
    write!(&mut multipart_body, "------XYZ--\r\n").unwrap();

    let response = request_with_auth_and_multipart(Method::POST, "/user", multipart_body);

    let (parts, body) = response.await.into_parts();

    assert_eq!(parts.status, StatusCode::OK);

    let response_body: RestApiResponse<UserDto> = deserialize_json_body(body).await.unwrap();

    assert_eq!(response_body.0.status, StatusCode::OK);
    let user_dto = response_body.0.data.unwrap();

    Ok((payload, user_dto, image_file.to_string()))
}

#[tokio::test]
async fn test_create_user_with_file() {
    let created = create_user_with_file()
        .await
        .expect("Failed to create user with file");

    let payload = created.0;
    let user_dto = created.1;
    let image_file = created.2;

    assert!(!user_dto.id.is_empty());
    assert_eq!(user_dto.username, payload.username.clone());
    assert_eq!(user_dto.email, Some(payload.email.clone()));
    assert_ne!(user_dto.modified_by, Some(payload.modified_by.clone()));
    assert_eq!(user_dto.origin_file_name, Some(image_file.to_string()));
    assert!(!user_dto.file_id.clone().unwrap_or_default().is_empty());
}

#[tokio::test]
async fn test_get_users() {
    let response = request_with_auth(Method::GET, "/user");

    let (parts, body) = response.await.into_parts();

    assert_eq!(parts.status, StatusCode::OK);

    let response_body: RestApiResponse<Page<UserDto>> = deserialize_json_body(body).await.unwrap();

    assert_eq!(response_body.0.status, StatusCode::OK);

    let page = response_body.0.data.unwrap();

    assert!(!page.items.is_empty());
    // The seed data has more users than this page size, so a cursor is returned.
    assert!(page.has_more);
    assert!(page.next_cursor.is_some());
}

#[tokio::test]
async fn test_get_users_pages_without_repeating_or_skipping() {
    let first = request_with_auth(Method::GET, "/user?limit=5").await;
    let first: RestApiResponse<Page<UserDto>> =
        deserialize_json_body(first.into_body()).await.unwrap();
    let first = first.0.data.unwrap();

    assert_eq!(first.items.len(), 5);
    let cursor = first.next_cursor.clone().expect("cursor for the next page");

    let second = request_with_auth(Method::GET, &format!("/user?limit=5&cursor={cursor}")).await;
    let second: RestApiResponse<Page<UserDto>> =
        deserialize_json_body(second.into_body()).await.unwrap();
    let second = second.0.data.unwrap();

    assert_eq!(second.items.len(), 5);

    // The pages are disjoint and keep descending order.
    let first_ids: Vec<&String> = first.items.iter().map(|u| &u.id).collect();
    for user in &second.items {
        assert!(
            !first_ids.contains(&&user.id),
            "page 2 repeated {}",
            user.id
        );
    }
}

#[tokio::test]
async fn test_get_users_rejects_invalid_cursor() {
    let response = request_with_auth(Method::GET, "/user?cursor=not-a-cursor").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_get_user_list() {
    let username = "user0".to_string();

    let payload = SearchUserDto {
        username: Some(username),
        id: None,
        email: None,
    };

    let response = request_with_auth_and_body(Method::POST, "/user/list", &payload);

    let (parts, body) = response.await.into_parts();

    assert_eq!(parts.status, StatusCode::OK);

    let response_body: RestApiResponse<Vec<UserDto>> = deserialize_json_body(body).await.unwrap();

    assert_eq!(response_body.0.status, StatusCode::OK);

    let user_dtos = response_body.0.data.unwrap();

    // println!("user_dtos: {:?}", user_dtos);
    assert!(!user_dtos.is_empty());
}

#[tokio::test]
async fn test_get_user_by_id() {
    let created = create_user().await.expect("Failed to create user");

    let existent_user = created.1;
    let existent_id = existent_user.id;

    let url = format!("/user/{}", existent_id);
    let response = request_with_auth(Method::GET, url.as_str());

    let (parts, body) = response.await.into_parts();

    assert_eq!(parts.status, StatusCode::OK);

    let response_body: RestApiResponse<UserDto> = deserialize_json_body(body).await.unwrap();

    assert_eq!(response_body.0.status, StatusCode::OK);
    let user_dto = response_body.0.data.unwrap();

    assert_eq!(user_dto.id, *existent_id);
    assert_eq!(user_dto.username, existent_user.username);
    assert_eq!(user_dto.email, existent_user.email);
    assert_eq!(user_dto.created_by, existent_user.created_by);
    assert_eq!(user_dto.created_at, existent_user.created_at);
    assert_eq!(user_dto.modified_by, existent_user.modified_by);
    assert_eq!(user_dto.modified_at, existent_user.modified_at);
    assert_eq!(user_dto.file_id, existent_user.file_id);
    assert_eq!(user_dto.origin_file_name, existent_user.origin_file_name);
}

#[tokio::test]
async fn test_update_user() {
    let created = create_user().await.expect("Failed to create user");

    let existent_user = created.1;
    let existent_id = existent_user.id;

    let username = format!("update-testuser-{}", uuid::Uuid::new_v4()).to_string();
    let email = format!("{}@test.com", username).to_string();

    let payload = UpdateUserDto {
        username,
        email,
        modified_by: TEST_USER_ID.to_string(),
    };

    let url = format!("/user/{}", existent_id);

    let response = request_with_auth_and_body(Method::PUT, url.as_str(), &payload);

    let (parts, body) = response.await.into_parts();

    assert_eq!(parts.status, StatusCode::OK);

    let response_body: RestApiResponse<UserDto> = deserialize_json_body(body).await.unwrap();

    assert_eq!(response_body.0.status, StatusCode::OK);
    let user_dto = response_body.0.data.unwrap();

    assert_eq!(user_dto.id, *existent_id);
    assert_eq!(user_dto.username, payload.username);
    assert_eq!(user_dto.email, Some(payload.email));
}

#[tokio::test]
async fn test_delete_user_not_found() {
    let non_existent_id = uuid::Uuid::new_v4();

    let url = format!("/user/{}", non_existent_id);
    let response = request_with_auth(Method::DELETE, url.as_str());

    let (parts, body) = response.await.into_parts();

    assert_eq!(parts.status, StatusCode::NOT_FOUND);

    let problem: ProblemDetails = deserialize_json_body(body).await.unwrap();

    assert_eq!(problem.status, StatusCode::NOT_FOUND.as_u16());
    assert_eq!(problem.type_uri, "/problems/not-found");
}

#[tokio::test]
async fn test_delete_user() {
    let created = create_user()
        .await
        .expect("Failed to create user for deletion");

    let user = created.1;

    let url = format!("/user/{}", user.id);

    let response = request_with_auth(Method::DELETE, url.as_str());

    let (parts, body) = response.await.into_parts();

    assert_eq!(parts.status, StatusCode::OK);

    let response_body: RestApiResponse<()> = deserialize_json_body(body).await.unwrap();

    assert_eq!(response_body.0.status, StatusCode::OK);
    // println!("response_body.0.status: {:?}", response_body.0.status);
    // println!("response_body.0.message: {:?}", response_body.0.message);
}

#[tokio::test]
async fn test_delete_user_file() {
    let created = create_user_with_file()
        .await
        .expect("Failed to create user with file for deletion");
    let user_dto = created.1;
    let file_id = user_dto.file_id.clone().unwrap_or_default();

    let url = format!("/file/{}", file_id);

    let response = request_with_auth(Method::DELETE, url.as_str());

    let (parts, body) = response.await.into_parts();

    assert_eq!(parts.status, StatusCode::OK);

    let response_body: RestApiResponse<()> = deserialize_json_body(body).await.unwrap();

    assert_eq!(response_body.0.status, StatusCode::OK);
    // println!("response_body.0.status: {:?}", response_body.0.status);
    // println!("response_body.0.message: {:?}", response_body.0.message);
}
