use axum::{
    body::{Body, Bytes},
    error_handling::HandleErrorLayer,
    extract::{DefaultBodyLimit, Request, State},
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        HeaderValue, Method, StatusCode,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use http_body_util::BodyExt;

use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    services::ServeDir,
    trace::TraceLayer,
};

use utoipa::OpenApi;

use crate::{
    common::{
        app_state::AppState,
        error::{handle_error, AppError},
        jwt,
    },
    domains::{
        auth::{user_auth_routes, UserAuthApiDoc},
        device::{device_routes, DeviceApiDoc},
        file::{file_routes, FileApiDoc},
        user::{user_routes, UserApiDoc},
    },
};

use utoipa_swagger_ui::SwaggerUi;

use std::sync::LazyLock;
use regex::Regex;

/// List of regex patterns representing disallowed content to block in requests.
/// These patterns are applied to both request bodies and URL query strings.
/// Used to detect and reject potentially dangerous input (e.g., script tags).
/// This is just sample. In real app this can be loaded from repository
pub static FORBIDDEN_PATTERNS: LazyLock<Vec<Regex>> =
    LazyLock::new(|| vec![Regex::new(r"(?i)<\s*script\b[^>]*>").unwrap()]);

fn create_swagger_ui() -> SwaggerUi {
    SwaggerUi::new("/docs")
        .url(
            "/api-docs/user_auth/openapi.json",
            UserAuthApiDoc::openapi(),
        )
        .url("/api-docs/user/openapi.json", UserApiDoc::openapi())
        .url("/api-docs/device/openapi.json", DeviceApiDoc::openapi())
        .url("/api-docs/file/openapi.json", FileApiDoc::openapi())
}

pub fn create_router(state: AppState) -> Router {
    // Build a CORS layer that applies to everyone
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_origin(cors_allow_origin(&state.config.cors_allowed_origins))
        .allow_headers([AUTHORIZATION, CONTENT_TYPE]);

    // Create a common middleware stack for error handling, timeouts, and CORS.
    let middleware_stack = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(handle_error))
        .timeout(Duration::from_secs(state.config.request_timeout_secs))
        .layer(cors);

    // /auth routes (login, register, refresh, etc.) — no logging here
    let auth_router = Router::new()
        .nest("/auth", user_auth_routes())
        .layer(middleware::from_fn(make_request_response_inspecter(false)));

    // Protected API routes
    let protected_routes = Router::new()
        .nest("/user", user_routes())
        .nest("/device", device_routes())
        .nest("/file", file_routes())
        // by default, Multipart limits to 2MB; override with `asset_max_size`
        // See https://docs.rs/axum/latest/axum/extract/struct.Multipart.html
        .layer(DefaultBodyLimit::max(state.config.asset_max_size))
        // enforce JWT authentication
        .route_layer(middleware::from_fn_with_state(
            state.jwt_keys.clone(),
            jwt::jwt_auth,
        ))
        // attach inspecter
        .layer(middleware::from_fn(make_request_response_inspecter(true)));

    // setup assets routes
    let public_assets_routes = Router::new().nest_service(
        state.config.assets_public_url.as_str(),
        ServeDir::new(state.config.assets_public_path.clone()),
    );

    let private_assets_routes = Router::new()
        .nest_service(
            state.config.assets_private_url.as_str(),
            ServeDir::new(state.config.assets_private_path.clone()),
        )
        // enforce JWT authentication
        .route_layer(middleware::from_fn_with_state(
            state.jwt_keys.clone(),
            jwt::jwt_auth,
        ))
        // attach inspecter
        .layer(middleware::from_fn(make_request_response_inspecter(true)));

    // Create the main router
    // and merge all the routes
    // and add the middleware stack
    // and add the state
    Router::new()
        .route("/health", axum::routing::get(health_check))
        .route("/ready", axum::routing::get(readiness_check))
        .merge(auth_router)
        .merge(protected_routes)
        .merge(create_swagger_ui())
        .merge(public_assets_routes)
        .merge(private_assets_routes)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &axum::http::Request<_>| {
                    tracing::info_span!(
                        "request",
                        method = %req.method(),
                        uri = %req.uri(),
                    )
                })
                .on_response(
                    |response: &axum::http::Response<_>,
                     latency: std::time::Duration,
                     _span: &tracing::Span| {
                        tracing::info!(
                            "request completed: status = {status}, latency = {latency:?}",
                            status = response.status(),
                            latency = latency
                        );
                    },
                ),
        )
        .fallback(fallback)
        .layer(middleware_stack)
        .with_state(state)
}

/// Builds the CORS origin policy from config. An empty list allows any origin.
fn cors_allow_origin(origins: &[String]) -> AllowOrigin {
    if origins.is_empty() {
        return AllowOrigin::from(Any);
    }
    let values: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|o| match HeaderValue::from_str(o) {
            Ok(v) => Some(v),
            Err(_) => {
                tracing::warn!("Ignoring invalid CORS origin {o:?}");
                None
            }
        })
        .collect();
    AllowOrigin::list(values)
}

/// Liveness probe: the process is up and serving HTTP.
async fn health_check() -> &'static str {
    "OK\n"
}

/// Readiness probe: the process can reach its backing database.
async fn readiness_check(State(state): State<AppState>) -> (StatusCode, &'static str) {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (StatusCode::OK, "READY\n"),
        Err(err) => {
            tracing::warn!("Readiness check failed: {err}");
            (StatusCode::SERVICE_UNAVAILABLE, "NOT READY\n")
        }
    }
}

/// Fallback handler for unmatched routes
/// This function returns a 404 Not Found response with a message.
pub async fn fallback() -> Result<impl IntoResponse, AppError> {
    Ok((StatusCode::NOT_FOUND, "Not Found"))
}

// Type alias for the boxed future returned by the request/response inspector middleware
type InspectorFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Response, (StatusCode, String)>> + Send>,
>;

/// Middleware that inspects request bodies and URL query strings, as well as response bodies, logging them for debugging, and rejects forbidden content.
/// Intercepts HTTP requests and responses: buffers bodies and query strings, then logs their content.
/// Returns a 403 Forbidden error if any forbidden patterns are detected in the request body or query string.
/// Note: multipart/form-data requests bypass this middleware and must be validated within their handlers.
fn make_request_response_inspecter(
    log_enabled: bool,
) -> impl Fn(Request<Body>, Next) -> InspectorFuture + Clone + Send + Sync + 'static {
    move |req, next| {
        let fut = request_response_inspecter(req, next, log_enabled);
        Box::pin(fut)
    }
}

async fn request_response_inspecter(
    req: Request<Body>,
    next: Next,
    log_enabled: bool,
) -> Result<Response, (StatusCode, String)> {
    // inspect forbidden query string
    if let Some(query) = req.uri().query() {
        if FORBIDDEN_PATTERNS.iter().any(|re| re.is_match(query)) {
            return Err((StatusCode::FORBIDDEN, "Forbidden Request".to_string()));
        }
    }

    let (parts, body) = req.into_parts();
    let bytes = request_inspect_print("request", log_enabled, body).await?;
    let req = Request::from_parts(parts, Body::from(bytes));

    let mut res = next.run(req).await;
    if log_enabled && tracing::enabled!(tracing::Level::DEBUG) {
        let (parts, body) = res.into_parts();
        let bytes = response_print("response", body).await?;
        res = Response::from_parts(parts, Body::from(bytes));
    }

    Ok(res)
}

/// This function inspects forbidden request and collects the body data into bytes and prints it to the log.
async fn request_inspect_print<B>(
    direction: &str,
    log_enabled: bool,
    body: B,
) -> Result<Bytes, (StatusCode, String)>
where
    B: axum::body::HttpBody<Data = Bytes>,
    B::Error: std::fmt::Display,
{
    let bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("failed to read {direction} body: {err}"),
            ));
        }
    };

    if let Ok(body_str) = std::str::from_utf8(&bytes) {
        // Bodies can contain personal data, so they are only logged at debug level.
        if log_enabled {
            tracing::debug!("{} body = {:?}", direction, truncate_for_log(body_str));
        }

        // inspect forbidden request body
        if FORBIDDEN_PATTERNS.iter().any(|re| re.is_match(body_str)) {
            return Err((StatusCode::FORBIDDEN, "Forbidden Request".to_string()));
        }
    }

    Ok(bytes)
}

async fn response_print<B>(direction: &str, body: B) -> Result<Bytes, (StatusCode, String)>
where
    B: axum::body::HttpBody<Data = Bytes>,
    B::Error: std::fmt::Display,
{
    let bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("failed to read {direction} body: {err}"),
            ));
        }
    };

    if let Ok(body_str) = std::str::from_utf8(&bytes) {
        tracing::debug!("{} body = {:?}", direction, truncate_for_log(body_str));
    }

    Ok(bytes)
}

/// Maximum number of bytes of a body written to the log.
const MAX_LOGGED_BODY_BYTES: usize = 4096;

/// Truncates a body to `MAX_LOGGED_BODY_BYTES`, respecting UTF-8 boundaries.
fn truncate_for_log(body: &str) -> &str {
    if body.len() <= MAX_LOGGED_BODY_BYTES {
        return body;
    }
    let mut end = MAX_LOGGED_BODY_BYTES;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    &body[..end]
}
