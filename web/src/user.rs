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
/// Credentials are stripped from the handler-facing user.
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
            .map(|user| {
                Self(User {
                    password_hash: String::new(),
                    sync_token: None,
                    sync_url: None,
                    ..user.0
                })
            })
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

    // AuthSession has private metadata, so middleware supplies the fixture without database I/O.
    async fn current_user_response(user: Option<super::AuthUser>) -> axum::response::Response {
        let pool = sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap();
        let store = crate::session_store::SqliteStore::new(pool.clone());
        let layer = axum_login::AuthManagerLayerBuilder::new(
            super::Backend::new(std::sync::Arc::new(pool)),
            tower_sessions::SessionManagerLayer::new(store),
        )
        .build();
        let handler = move |mut auth_session: super::AuthSession,
                            request: axum::extract::Request| async move {
            auth_session.user = user;
            let (mut parts, _) = request.into_parts();
            parts.extensions.insert(auth_session);
            let extracted =
                <super::CurrentUser as axum::extract::FromRequestParts<()>>::from_request_parts(
                    &mut parts,
                    &(),
                )
                .await;
            if let Ok(super::CurrentUser(user)) = &extracted {
                assert!(
                    user.password_hash.is_empty(),
                    "extractor exposed password hash"
                );
                assert!(user.sync_url.is_none(), "extractor exposed sync url");
                assert!(user.sync_token.is_none(), "extractor exposed sync token");
            }

            axum::response::IntoResponse::into_response(
                extracted.map(|super::CurrentUser(user)| axum::Json(user)),
            )
        };

        axum::handler::Handler::call(
            axum::handler::Handler::layer(handler, layer),
            axum::http::Request::new(axum::body::Body::empty()),
            (),
        )
        .await
    }

    #[tokio::test]
    async fn current_user_extractor_preserves_identity_and_redacts_credentials() {
        for bits in 0..=UserPermissions::all().bits() {
            let permissions = UserPermissions::from_bits(bits).expect("bits should be known flags");
            let auth_user = super::AuthUser(stored_user(permissions, Some("sync-url-secret")));
            assert_eq!(axum_login::AuthUser::id(&auth_user), 7);

            let response = current_user_response(Some(auth_user)).await;

            assert_eq!(response.status(), axum::http::StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let user: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(
                user,
                serde_json::json!({
                    "id": 7,
                    "username": "alice",
                    "email": "alice@example.com",
                    "created_at": "2024-01-02 03:04:05",
                    "last_sync": "2024-05-06 07:08:09",
                    "permissions": permissions,
                    "sync_url": null,
                    "sync_token": null,
                })
            );
        }
    }

    #[tokio::test]
    async fn current_user_extractor_rejects_unauthenticated_session() {
        let response = current_user_response(None).await;

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"Not logged in");
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
