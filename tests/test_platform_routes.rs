use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use tower::ServiceExt;

mod test_helpers;

use test_helpers::{create_test_router, request, request_with_body};

#[tokio::test]
async fn test_ready_checks_database() {
    let response = request(Method::GET, "/ready").await;

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_response_has_generated_request_id() {
    let response = request(Method::GET, "/health").await;

    let request_id = response
        .headers()
        .get("x-request-id")
        .expect("x-request-id header missing");
    assert!(!request_id.is_empty());
}

#[tokio::test]
async fn test_incoming_request_id_is_propagated() {
    let app = create_test_router().await;
    let request = Request::builder()
        .uri("/health")
        .header("x-request-id", "test-correlation-id")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(
        response.headers().get("x-request-id").unwrap(),
        "test-correlation-id"
    );
}

#[tokio::test]
async fn test_oversized_json_body_is_rejected_before_buffering() {
    // Larger than the inspector's 2MB cap for non-multipart bodies.
    let payload = "a".repeat(3 * 1024 * 1024);

    let response = request_with_body(Method::POST, "/device", &payload).await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
