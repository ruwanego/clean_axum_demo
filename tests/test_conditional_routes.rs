//! Conditional request behaviour: ETag, If-None-Match and If-Match.

use axum::http::{Method, StatusCode, header};

use clean_axum_demo::common::{dto::RestApiResponse, problem::ProblemDetails};
use clean_axum_demo::domains::user::dto::user_dto::UserDto;

mod test_helpers;

use test_helpers::{
    TEST_USER_ID, deserialize_json_body, request_with_auth, request_with_auth_and_headers,
    request_with_auth_and_multipart,
};

/// Creates a user of its own, so tests never contend for the same row.
async fn create_user() -> UserDto {
    let username = format!("etag-{}", uuid::Uuid::new_v4());
    let multipart_body = format!(
        "------XYZ\r\nContent-Disposition: form-data; name=\"username\"\r\n\r\n{username}\r\n------XYZ\r\nContent-Disposition: form-data; name=\"email\"\r\n\r\n{username}@test.com\r\n------XYZ\r\nContent-Disposition: form-data; name=\"modified_by\"\r\n\r\n{TEST_USER_ID}\r\n------XYZ--\r\n"
    )
    .into_bytes();

    let response = request_with_auth_and_multipart(Method::POST, "/user", multipart_body).await;
    assert_eq!(response.status(), StatusCode::OK);

    let body: RestApiResponse<UserDto> = deserialize_json_body(response.into_body()).await.unwrap();
    body.0.data.unwrap()
}

/// Reads a user and its ETag.
async fn get_user_with_etag(id: &str) -> (UserDto, String) {
    let response = request_with_auth(Method::GET, &format!("/user/{id}")).await;
    assert_eq!(response.status(), StatusCode::OK);

    let etag = response
        .headers()
        .get(header::ETAG)
        .expect("ETag header")
        .to_str()
        .unwrap()
        .to_string();

    let body: RestApiResponse<UserDto> = deserialize_json_body(response.into_body()).await.unwrap();

    (body.0.data.unwrap(), etag)
}

fn update_payload(user: &UserDto) -> serde_json::Value {
    serde_json::json!({
        "username": user.username,
        "email": user.email,
        "modified_by": TEST_USER_ID,
    })
}

#[tokio::test]
async fn get_returns_etag_and_304_for_a_fresh_copy() {
    let user = create_user().await;
    let (_, etag) = get_user_with_etag(&user.id).await;

    let response = request_with_auth_and_headers::<()>(
        Method::GET,
        &format!("/user/{}", user.id),
        &[("if-none-match", etag)],
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn update_with_a_stale_if_match_is_rejected() {
    let user = create_user().await;

    let response = request_with_auth_and_headers(
        Method::PUT,
        &format!("/user/{}", user.id),
        &[("if-match", "\"stale-etag\"".to_string())],
        Some(&update_payload(&user)),
    )
    .await;

    assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);

    let problem: ProblemDetails = deserialize_json_body(response.into_body()).await.unwrap();
    assert_eq!(problem.type_uri, "/problems/precondition-failed");
}

#[tokio::test]
async fn update_with_the_current_if_match_succeeds_and_returns_a_new_etag() {
    let created = create_user().await;
    let (user, etag) = get_user_with_etag(&created.id).await;

    let response = request_with_auth_and_headers(
        Method::PUT,
        &format!("/user/{}", user.id),
        &[("if-match", etag.clone())],
        Some(&update_payload(&user)),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);

    let new_etag = response
        .headers()
        .get(header::ETAG)
        .expect("ETag header")
        .to_str()
        .unwrap()
        .to_string();

    // modified_at moved on, so the tag must differ: a client holding the old one
    // can no longer overwrite this row.
    assert_ne!(new_etag, etag);
}

#[tokio::test]
async fn update_without_if_match_is_still_allowed() {
    let user = create_user().await;

    let response = request_with_auth_and_headers(
        Method::PUT,
        &format!("/user/{}", user.id),
        &[],
        Some(&update_payload(&user)),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
}
