use std::{fmt, sync::Arc};

use axum::{extract::FromRequestParts, http::StatusCode, http::request::Parts};
use axum_login::{AuthnBackend, UserId};
use password_auth::verify_password;
use serde::Deserialize;
use tokio::task;

use db::{db_context::DbContext, model::user::User, user_db};

use crate::error::ApplicationError;

#[derive(Clone)]
pub struct AuthUser(User);

// `User`'s derived `Debug` would log the password hash, sync token and sync url.
impl fmt::Debug for AuthUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("User")
            .field("id", &self.0.id)
            .field("username", &self.0.username)
            .field("email", &self.0.email)
            .field("permissions", &self.0.permissions)
            .finish_non_exhaustive()
    }
}

impl axum_login::AuthUser for AuthUser {
    type Id = i64;

    fn id(&self) -> Self::Id {
        self.0.id
    }

    /// The sync url doubles as the validity token: `scramble_validity_token`
    /// rewrites it, which invalidates every existing session of the user.
    fn session_auth_hash(&self) -> &[u8] {
        self.0.sync_url.as_deref().unwrap_or_default().as_bytes()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub enum Credentials {
    Password(PasswordCredentials),
    Token(TokenCredentials),
}

#[derive(Debug, Clone, Deserialize)]
pub struct PasswordCredentials {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenCredentials {
    pub token: String,
}

#[derive(Debug, Clone)]
pub struct Backend {
    db: Arc<DbContext>,
}

impl Backend {
    pub const fn new(db: Arc<DbContext>) -> Self {
        Self { db }
    }
}

impl AuthnBackend for Backend {
    type User = AuthUser;
    type Credentials = Credentials;
    type Error = ApplicationError;

    async fn authenticate(
        &self,
        creds: Self::Credentials,
    ) -> Result<Option<Self::User>, Self::Error> {
        match creds {
            Credentials::Password(password_credentials) => {
                let user =
                    user_db::get_user_by_email(&self.db, &password_credentials.email).await?;

                let user = task::spawn_blocking(|| {
                    user.filter(|user| {
                        verify_password(password_credentials.password, &user.password_hash).is_ok()
                    })
                })
                .await?;

                Ok(user.map(AuthUser))
            }
            Credentials::Token(token_credentials) => {
                let user =
                    user_db::get_user_by_sync_token(&self.db, &token_credentials.token).await?;

                Ok(user.map(AuthUser))
            }
        }
    }

    async fn get_user(&self, user_id: &UserId<Self>) -> Result<Option<Self::User>, Self::Error> {
        let user = user_db::get_user_by_id(&self.db, *user_id).await?;

        Ok(user.map(AuthUser))
    }
}

pub type AuthSession = axum_login::AuthSession<Backend>;

/// Extracts the domain [`User`] for the authenticated session, so handlers
/// don't each repeat the auth-session unwrap. Rejects
/// with 401 when no user is logged in (matching `login_required!` semantics;
/// the route layers stay in place as defense in depth).
pub struct CurrentUser(pub User);

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let auth_session = AuthSession::from_request_parts(parts, state).await?;

        auth_session
            .user
            .map(|user| Self(user.0))
            .ok_or((StatusCode::UNAUTHORIZED, "Not logged in"))
    }
}

#[cfg(test)]
mod tests {
    use db::model::user::UserPermissions;

    fn stored_user(permissions: UserPermissions, sync_url: Option<&str>) -> db::model::user::User {
        db::model::user::User {
            id: 7,
            username: "alice".to_string(),
            email: "alice@example.com".to_string(),
            created_at: "2024-01-02 03:04:05".to_string(),
            sync_url: sync_url.map(str::to_string),
            sync_token: Some("sync-token-secret".to_string()),
            last_sync: Some("2024-05-06 07:08:09".to_string()),
            permissions,
            password_hash: "$argon2id$v=19$password-hash-secret".to_string(),
        }
    }

    #[test]
    fn stored_user_round_trip_preserves_identity_and_every_permission_combination() {
        for bits in 0..=UserPermissions::all().bits() {
            let permissions = UserPermissions::from_bits(bits).expect("bits should be known flags");
            let auth_user = super::AuthUser(stored_user(permissions, None));
            assert_eq!(axum_login::AuthUser::id(&auth_user), 7);

            let user = auth_user.0;

            assert_eq!(
                (
                    user.id,
                    user.username.as_str(),
                    user.email.as_str(),
                    user.permissions
                ),
                (7, "alice", "alice@example.com", permissions)
            );
        }
    }

    /// Existing sessions stay valid only while this hash is byte-for-byte unchanged.
    #[test]
    fn session_auth_hash_is_the_sync_url_bytes() {
        let with_sync_url = super::AuthUser(stored_user(
            UserPermissions::Sync,
            Some("https://sync.example/user-7"),
        ));
        let without_sync_url = super::AuthUser(stored_user(UserPermissions::Sync, None));

        assert_eq!(
            axum_login::AuthUser::session_auth_hash(&with_sync_url),
            b"https://sync.example/user-7"
        );
        assert_eq!(
            axum_login::AuthUser::session_auth_hash(&without_sync_url),
            b""
        );
    }

    #[test]
    fn debug_output_redacts_credentials() {
        let auth_user =
            super::AuthUser(stored_user(UserPermissions::Admin, Some("sync-url-secret")));

        let debug = format!("{auth_user:?}");

        assert!(debug.contains("alice"), "unexpected Debug output: {debug}");
        for secret in [
            "password-hash-secret",
            "sync-token-secret",
            "sync-url-secret",
        ] {
            assert!(!debug.contains(secret), "Debug output leaks a secret field");
        }
    }
}
