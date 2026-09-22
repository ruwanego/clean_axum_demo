use regex::Regex;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::env;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

/// Config is a struct that holds the configuration for the application.
/// All values come from environment variables (12-factor: config in the environment).
#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub database_max_connections: u32,
    pub database_min_connections: u32,
    pub database_connect_retries: u32,
    pub database_acquire_timeout_secs: u64,

    pub service_host: String,
    pub service_port: u16,

    pub jwt_secret: String,
    pub jwt_expiry_secs: i64,

    pub request_timeout_secs: u64,
    /// Allowed CORS origins. An empty list means any origin (`*`).
    pub cors_allowed_origins: Vec<String>,

    pub assets_public_path: String,
    pub assets_public_url: String,

    pub assets_private_path: String,
    pub assets_private_url: String,

    pub asset_allowed_extensions_pattern: Regex,
    pub asset_max_size: usize,
}

/// ConfigError names the environment variable that is missing or invalid.
#[derive(Debug)]
pub struct ConfigError {
    pub var: &'static str,
    pub reason: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "config error in {}: {}", self.var, self.reason)
    }
}

impl std::error::Error for ConfigError {}

/// Reads a required environment variable.
fn required(var: &'static str) -> Result<String, ConfigError> {
    env::var(var).map_err(|e| ConfigError {
        var,
        reason: e.to_string(),
    })
}

/// Reads an optional environment variable, parsing it when present.
/// A present-but-invalid value is an error rather than a silent fallback.
fn parsed_or<T>(var: &'static str, default: T) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    match env::var(var) {
        Ok(s) => s.trim().parse::<T>().map_err(|e| ConfigError {
            var,
            reason: format!("invalid value {s:?}: {e}"),
        }),
        Err(_) => Ok(default),
    }
}

/// Reads a required environment variable and parses it.
fn required_parsed<T>(var: &'static str) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    let s = required(var)?;
    s.trim().parse::<T>().map_err(|e| ConfigError {
        var,
        reason: format!("invalid value {s:?}: {e}"),
    })
}

/// from_env reads the environment variables and returns a Config struct.
/// It does not load any `.env` file itself; that is done once in `main` for local development.
impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let ext_val = required("ASSET_ALLOWED_EXTENSIONS")?;
        let asset_allowed_extensions_pattern = Regex::new(&format!(r"(?i)^.*\.({})$", ext_val))
            .map_err(|e| ConfigError {
                var: "ASSET_ALLOWED_EXTENSIONS",
                reason: e.to_string(),
            })?;

        let jwt_secret = required("JWT_SECRET_KEY")?;
        if jwt_secret.len() < 32 {
            return Err(ConfigError {
                var: "JWT_SECRET_KEY",
                reason: "must be at least 32 characters".into(),
            });
        }

        let cors_allowed_origins = env::var("CORS_ALLOWED_ORIGINS")
            .unwrap_or_else(|_| "*".into())
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "*")
            .map(String::from)
            .collect();

        Ok(Self {
            database_url: required("DATABASE_URL")?,
            database_max_connections: parsed_or("DATABASE_MAX_CONNECTIONS", 5)?,
            database_min_connections: parsed_or("DATABASE_MIN_CONNECTIONS", 1)?,
            database_connect_retries: parsed_or("DATABASE_CONNECT_RETRIES", 5)?,
            database_acquire_timeout_secs: parsed_or("DATABASE_ACQUIRE_TIMEOUT_SECS", 5)?,

            service_host: env::var("SERVICE_HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            service_port: required_parsed("SERVICE_PORT")?,

            jwt_secret,
            jwt_expiry_secs: parsed_or("JWT_EXPIRY_SECS", 24 * 60 * 60)?,

            request_timeout_secs: parsed_or("REQUEST_TIMEOUT_SECS", 1800)?,
            cors_allowed_origins,

            assets_public_path: required("ASSETS_PUBLIC_PATH")?,
            assets_public_url: required("ASSETS_PUBLIC_URL")?,

            assets_private_path: required("ASSETS_PRIVATE_PATH")?,
            assets_private_url: required("ASSETS_PRIVATE_URL")?,

            asset_allowed_extensions_pattern,
            asset_max_size: parsed_or("ASSET_MAX_SIZE", 50 * 1024 * 1024)?, // Default to 50MB
        })
    }
}

/// Debug is implemented by hand so secrets never end up in logs.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("database_url", &redact_url_password(&self.database_url))
            .field("database_max_connections", &self.database_max_connections)
            .field("database_min_connections", &self.database_min_connections)
            .field("database_connect_retries", &self.database_connect_retries)
            .field(
                "database_acquire_timeout_secs",
                &self.database_acquire_timeout_secs,
            )
            .field("service_host", &self.service_host)
            .field("service_port", &self.service_port)
            .field("jwt_secret", &"<redacted>")
            .field("jwt_expiry_secs", &self.jwt_expiry_secs)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("cors_allowed_origins", &self.cors_allowed_origins)
            .field("assets_public_path", &self.assets_public_path)
            .field("assets_public_url", &self.assets_public_url)
            .field("assets_private_path", &self.assets_private_path)
            .field("assets_private_url", &self.assets_private_url)
            .field(
                "asset_allowed_extensions_pattern",
                &self.asset_allowed_extensions_pattern.as_str(),
            )
            .field("asset_max_size", &self.asset_max_size)
            .finish()
    }
}

/// Replaces the password part of `scheme://user:password@host/...` with `***`.
fn redact_url_password(url: &str) -> String {
    let Some(scheme_end) = url.find("://").map(|i| i + 3) else {
        return url.to_string();
    };
    let Some(at) = url[scheme_end..].find('@').map(|i| i + scheme_end) else {
        return url.to_string();
    };
    match url[scheme_end..at].find(':').map(|i| i + scheme_end) {
        Some(colon) => format!("{}:***{}", &url[..colon], &url[at..]),
        None => url.to_string(),
    }
}

/// setup_database initializes the database connection pool.
/// Retries with exponential backoff so the process tolerates a backing service that starts slowly.
pub async fn setup_database(config: &Config) -> Result<PgPool, sqlx::Error> {
    let max_attempts = config.database_connect_retries.max(1);
    let mut delay = Duration::from_millis(500);
    let mut attempts = 0;
    loop {
        attempts += 1;
        match PgPoolOptions::new()
            .max_connections(config.database_max_connections)
            .min_connections(config.database_min_connections)
            .acquire_timeout(Duration::from_secs(config.database_acquire_timeout_secs))
            .connect(&config.database_url)
            .await
        {
            Ok(pool) => return Ok(pool),
            Err(err) => {
                if attempts >= max_attempts {
                    return Err(err);
                }
                tracing::warn!(
                    "Postgres not ready yet ({err}), retrying in {delay:?} (attempt {attempts}/{max_attempts})"
                );
                sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(10));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::redact_url_password;

    #[test]
    fn redacts_password() {
        assert_eq!(
            redact_url_password("postgres://user:secret@localhost:5432/db"),
            "postgres://user:***@localhost:5432/db"
        );
        assert_eq!(
            redact_url_password("postgres://localhost/db"),
            "postgres://localhost/db"
        );
    }
}
