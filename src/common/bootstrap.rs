use std::sync::Arc;

use sqlx::PgPool;

use crate::common::config::Config;
use crate::common::jwt::Keys;
use crate::domains::auth::{AuthService, AuthServiceTrait};
use crate::domains::device::{DeviceService, DeviceServiceTrait};
use crate::domains::file::{FileService, FileServiceTrait};
use crate::domains::user::UserServiceTrait;
use crate::{common::app_state::AppState, domains::user::UserService};

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Constructs and wires all application services and returns a configured AppState.
pub fn build_app_state(pool: PgPool, config: Config) -> AppState {
    let jwt_keys = Keys::from_config(&config);
    let auth_service: Arc<dyn AuthServiceTrait> =
        AuthService::create_service(pool.clone(), Arc::clone(&jwt_keys));
    let file_service: Arc<dyn FileServiceTrait> =
        FileService::create_service(config.clone(), pool.clone());
    let user_service: Arc<dyn UserServiceTrait> =
        UserService::create_service(pool.clone(), Arc::clone(&file_service));
    let device_service: Arc<dyn DeviceServiceTrait> = DeviceService::create_service(pool.clone());

    AppState::new(
        config,
        pool,
        jwt_keys,
        auth_service,
        user_service,
        device_service,
        file_service,
    )
}

/// Loads a `.env` file into the process environment if one exists.
/// Intended for local development only: in deployed environments the real
/// environment is the single source of config and no `.env` file is present.
/// Variables already set in the environment are never overridden.
pub fn load_dotenv() {
    if let Ok(path) = dotenvy::dotenv() {
        eprintln!("Loaded environment from {}", path.display());
    }
}

/// Returns the log filter from `RUST_LOG`, or a sensible default.
fn env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,sqlx=info,tower_http=info,axum::rejection=trace".into())
}

/// Setup tracing for the application.
/// Logs are written to stdout as an event stream. `LOG_FORMAT=json` emits one JSON
/// object per line for log aggregators; anything else uses the human-readable format.
pub fn setup_tracing() {
    let json = std::env::var("LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    let registry = tracing_subscriber::registry().with(env_filter());

    if json {
        registry
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_current_span(true)
                    .with_span_list(false)
                    .with_target(true),
            )
            .init();
    } else {
        registry
            .with(
                tracing_subscriber::fmt::layer()
                    .with_file(true)
                    .with_line_number(true)
                    .with_thread_ids(true)
                    .with_thread_names(true)
                    .with_target(true)
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE),
            )
            .init();
    }
}

/// Shutdown signal handler.
/// Completes on CTRL+C or, on Unix, SIGTERM (sent by `docker stop` and Kubernetes),
/// so in-flight requests are drained before the process exits.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install CTRL+C signal handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Received CTRL+C, shutting down gracefully"),
        _ = terminate => tracing::info!("Received SIGTERM, shutting down gracefully"),
    }
}
