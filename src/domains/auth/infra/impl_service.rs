use std::sync::Arc;

use crate::{
    common::{
        error::AppError,
        hash_util,
        jwt::{AuthBody, AuthPayload, Keys, make_jwt_token},
    },
    domains::auth::{
        domain::{model::UserAuth, repository::UserAuthRepository, service::AuthServiceTrait},
        dto::auth_dto::AuthUserDto,
        infra::impl_repository::UserAuthRepo,
    },
};

use sqlx::PgPool;

/// Service for handling user authentication
/// and authorization logic.
#[derive(Clone)]
pub struct AuthService {
    pool: PgPool,
    repo: Arc<dyn UserAuthRepository + Send + Sync>,
    keys: Arc<Keys>,
}

/// Implementation of the AuthService
#[async_trait::async_trait]
impl AuthServiceTrait for AuthService {
    /// constructor for the service.
    fn create_service(pool: PgPool, keys: Arc<Keys>) -> Arc<dyn AuthServiceTrait> {
        Arc::new(Self {
            pool,
            repo: Arc::new(UserAuthRepo {}),
            keys,
        })
    }

    /// It hashes the password and stores it in the database.
    async fn create_user_auth(&self, auth_user: AuthUserDto) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;

        let password_hash =
            hash_util::hash_password(&auth_user.password).map_err(|_| AppError::InternalError)?;

        let user_auth = UserAuth {
            user_id: auth_user.user_id,
            password_hash,
        };

        match self.repo.create(&mut tx, user_auth).await {
            Ok(()) => {
                tx.commit().await?;
                Ok(())
            }
            Err(err) => {
                tracing::error!("Error creating user auth: {err}");
                tx.rollback().await?;
                Err(AppError::DatabaseError(err))
            }
        }
    }

    /// Authenticates a user by checking the provided credentials
    /// against the stored credentials in the database.
    /// If the credentials are valid, it generates a JWT token for the user.
    /// If the credentials are invalid, it returns an error.
    async fn login_user(&self, auth_payload: AuthPayload) -> Result<AuthBody, AppError> {
        if auth_payload.client_id.is_empty() || auth_payload.client_secret.is_empty() {
            return Err(AppError::MissingCredentials);
        }

        let user_auth = self
            .repo
            .find_by_user_name(self.pool.clone(), auth_payload.client_id.clone())
            .await
            .map_err(AppError::DatabaseError)?;

        let user_auth = user_auth.ok_or(AppError::UserNotFound)?;

        if !hash_util::verify_password(&user_auth.password_hash, &auth_payload.client_secret) {
            return Err(AppError::WrongCredentials);
        }

        let token =
            make_jwt_token(&self.keys, &user_auth.user_id).map_err(|_| AppError::InternalError)?;

        Ok(AuthBody::new(token))
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the auth use cases. The repository is replaced by an
    //! in-memory fake, and the pool is lazy (never connected), so no database is needed.

    use super::*;
    use crate::common::jwt::Claims;
    use jsonwebtoken::{Validation, decode};
    use sqlx::{Postgres, Transaction, postgres::PgPoolOptions};

    const USER_NAME: &str = "alice";
    const USER_ID: &str = "00000000-0000-0000-0000-00000000000a";
    const PASSWORD: &str = "correct horse battery staple";

    struct FakeUserAuthRepo {
        user: Option<UserAuth>,
    }

    #[async_trait::async_trait]
    impl UserAuthRepository for FakeUserAuthRepo {
        async fn find_by_user_name(
            &self,
            _pool: PgPool,
            user_name: String,
        ) -> Result<Option<UserAuth>, sqlx::Error> {
            Ok(self.user.clone().filter(|_| user_name == USER_NAME))
        }

        async fn create(
            &self,
            _tx: &mut Transaction<'_, Postgres>,
            _user_auth: UserAuth,
        ) -> Result<(), sqlx::Error> {
            Ok(()) // not used by these tests
        }
    }

    fn keys() -> Arc<Keys> {
        Arc::new(Keys::new(
            b"unit-test-secret-unit-test-secret",
            chrono::Duration::minutes(5),
        ))
    }

    fn service(user: Option<UserAuth>) -> AuthService {
        AuthService {
            pool: PgPoolOptions::new()
                .connect_lazy("postgres://unused@localhost/unused")
                .expect("lazy pool"),
            repo: Arc::new(FakeUserAuthRepo { user }),
            keys: keys(),
        }
    }

    fn stored_user() -> UserAuth {
        UserAuth {
            user_id: USER_ID.into(),
            password_hash: hash_util::hash_password(PASSWORD).unwrap(),
        }
    }

    fn payload(client_id: &str, client_secret: &str) -> AuthPayload {
        AuthPayload {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
        }
    }

    #[tokio::test]
    async fn login_returns_token_for_the_user() {
        let body = service(Some(stored_user()))
            .login_user(payload(USER_NAME, PASSWORD))
            .await
            .expect("login should succeed");

        assert_eq!(body.token_type, "Bearer");
        let claims = decode::<Claims>(&body.access_token, &keys().decoding, &Validation::default())
            .expect("token should verify")
            .claims;
        assert_eq!(claims.sub, USER_ID);
        assert!(claims.exp - claims.iat <= 5 * 60, "uses configured expiry");
    }

    #[tokio::test]
    async fn login_rejects_wrong_password() {
        let result = service(Some(stored_user()))
            .login_user(payload(USER_NAME, "wrong"))
            .await;

        assert!(matches!(result, Err(AppError::WrongCredentials)));
    }

    #[tokio::test]
    async fn login_rejects_unknown_user() {
        let result = service(None).login_user(payload(USER_NAME, PASSWORD)).await;

        assert!(matches!(result, Err(AppError::UserNotFound)));
    }

    #[tokio::test]
    async fn login_rejects_missing_credentials() {
        let service = service(Some(stored_user()));

        for (id, secret) in [("", PASSWORD), (USER_NAME, "")] {
            let result = service.login_user(payload(id, secret)).await;
            assert!(matches!(result, Err(AppError::MissingCredentials)));
        }
    }
}
