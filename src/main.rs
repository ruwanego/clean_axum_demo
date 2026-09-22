use clean_axum_demo::{app::create_router, common};
use common::{
    bootstrap::{build_app_state, load_dotenv, shutdown_signal},
    config::{Config, setup_database},
};
use sqlx::PgPool;
use tracing::info;

#[cfg(not(feature = "opentelemetry"))]
use common::bootstrap::setup_tracing;

#[cfg(feature = "opentelemetry")]
use common::opentelemetry::{setup_tracing_opentelemetry, shutdown_opentelemetry};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Main entry point for the application.
///
/// Usage:
/// - `clean_axum_demo` (or `clean_axum_demo serve`) runs the web server.
/// - `clean_axum_demo migrate` applies pending database migrations and exits.
///   This is a one-off admin process run from the same release as the server.
///
/// # Errors
/// Returns an error if configuration is invalid, the database connection fails,
/// or the server fails to start.
#[tokio::main]
async fn main() -> Result<(), BoxError> {
    // Local development convenience only; deployed environments set real env vars.
    load_dotenv();

    #[cfg(not(feature = "opentelemetry"))]
    setup_tracing();

    #[cfg(feature = "opentelemetry")]
    let opentelemetry_tracer_provider = {
        let provider = setup_tracing_opentelemetry();
        // Startup span to ensure at least one span is generated and exported
        let span = tracing::info_span!("startup");
        let _enter = span.enter();
        provider
    };

    let command = std::env::args().nth(1).unwrap_or_else(|| "serve".into());

    let config = Config::from_env()?;
    let pool = setup_database(&config).await?;

    let result = match command.as_str() {
        "serve" => serve(pool, config).await,
        "migrate" => migrate(&pool).await,
        other => {
            Err(format!("unknown command {other:?} (expected \"serve\" or \"migrate\")").into())
        }
    };

    #[cfg(feature = "opentelemetry")]
    shutdown_opentelemetry(opentelemetry_tracer_provider)?;

    result
}

/// Runs the HTTP server until a shutdown signal is received.
async fn serve(pool: PgPool, config: Config) -> Result<(), BoxError> {
    if env_flag("RUN_MIGRATIONS_ON_START") {
        migrate(&pool).await?;
    }

    let state = build_app_state(pool.clone(), config.clone());
    let app = create_router(state);

    let addr = format!("{}:{}", config.service_host, config.service_port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    info!("Server running at {addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Server stopped, closing database pool");
    pool.close().await;

    Ok(())
}

/// Applies pending migrations from the `migrations/` directory (embedded at compile time).
async fn migrate(pool: &PgPool) -> Result<(), BoxError> {
    info!("Running database migrations");
    sqlx::migrate!("./migrations").run(pool).await?;
    info!("Database migrations applied");
    Ok(())
}

fn env_flag(var: &str) -> bool {
    std::env::var(var)
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}
