use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::sync::Arc;
use utoipa::ToSchema;

use super::{config::Config, error::AppError};

/// Keys is a struct that holds the encoding and decoding keys for JWT,
/// plus the token lifetime. It is built from `Config` (JWT_SECRET_KEY, JWT_EXPIRY_SECS).
pub struct Keys {
    pub encoding: EncodingKey,
    pub decoding: DecodingKey,
    pub expiry: Duration,
}

impl Keys {
    /// Builds the keys from the application config.
    pub fn from_config(config: &Config) -> Arc<Self> {
        let mut keys = Self::new(config.jwt_secret.as_bytes());
        keys.expiry = Duration::seconds(config.jwt_expiry_secs);
        Arc::new(keys)
    }
}

/// The Keys struct is used to create the encoding and decoding keys for JWT.
impl Keys {
    fn new(secret: &[u8]) -> Self {
        Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
            expiry: Duration::hours(24),
        }
    }
}

/// Claims is a struct that represents the claims in the JWT token.
/// It contains the subject (user ID), expiration time, and issued at time.
/// The `sub` field is the user ID, `exp` is the expiration time, and `iat` is the issued at time.
/// The `Claims` struct is used to encode and decode the JWT tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub iat: usize,
}

/// The Claims struct implements the `Display` trait for easy printing.
/// It formats the claims as a string, showing the user ID.
impl Display for Claims {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "user_id: {}", self.sub)
    }
}

/// The Default trait is implemented for the Claims struct.
/// It sets the default values for the claims.
impl Default for Claims {
    fn default() -> Self {
        let now = Utc::now();
        let expire: Duration = Duration::hours(24);
        let exp: usize = (now + expire).timestamp() as usize;
        let iat: usize = now.timestamp() as usize;
        Claims {
            sub: String::new(),
            exp,
            iat,
        }
    }
}

/// AuthBody is a struct that represents the authentication body.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AuthBody {
    pub access_token: String,
    pub token_type: String,
}

/// The AuthBody struct is used to create a new instance of the authentication body.
/// It takes an access token as a parameter and sets the token type to "Bearer".
impl AuthBody {
    pub fn new(access_token: String) -> Self {
        Self {
            access_token,
            token_type: "Bearer".to_string(),
        }
    }
}

/// AuthPayload is a struct that represents the authentication payload.
/// It contains the client ID and client secret.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AuthPayload {
    pub client_id: String,
    pub client_secret: String,
}

/// make_jwt_token is a function that creates a JWT token.
/// It takes the keys and a user ID and returns a Result with the JWT token or an error.
pub fn make_jwt_token(keys: &Keys, user_id: &str) -> Result<String, AppError> {
    let now = Utc::now();
    let claims = Claims {
        sub: user_id.to_string(),
        exp: (now + keys.expiry).timestamp() as usize,
        iat: now.timestamp() as usize,
    };
    encode(&Header::default(), &claims, &keys.encoding).map_err(|_| AppError::TokenCreation)
}

/// Middleware to validate JWT tokens.
/// If the token is valid, the request proceeds; otherwise, a 401 Unauthorized is returned.
/// Attach with `middleware::from_fn_with_state(keys, jwt_auth)`.
pub async fn jwt_auth(
    State(keys): State<Arc<Keys>>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    // Try to extract and trim the token in one go.
    let token = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|header| header.strip_prefix("Bearer "))
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .ok_or(AppError::InvalidToken)?;

    // Validate and decode the token.
    let token_data =
        decode::<Claims>(token, &keys.decoding, &Validation::default()).map_err(|err| {
            tracing::warn!("Error decoding token: {:?}", err);
            AppError::InvalidToken
        })?;

    // Insert the decoded claims into the request extensions.
    req.extensions_mut().insert(token_data.claims);
    Ok(next.run(req).await)
}
