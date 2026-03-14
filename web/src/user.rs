use std::sync::Arc;

use axum_login::{AuthUser as AxumAuthUser, AuthnBackend, UserId};
use password_auth::verify_password;
use serde::{Deserialize, Serialize};
use tokio::task;

use db::{db_context::DbContext, model::user::User, user_db};

use crate::error::ApplicationError;

#[derive(Clone, Serialize, Deserialize)]
pub struct AuthUser {
    id: i64,
    pub username: String,
    pub email: String,
    pub permissions: usize,
    pub validity_token: Vec<u8>,
}

// Here we've implemented `Debug` manually to avoid accidentally logging the
// password hash and validity token.
impl std::fmt::Debug for AuthUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("User")
            .field("id", &self.id)
            .field("username", &self.username)
            .field("email", &self.email)
            .field("permissions", &self.permissions)
            .field("validity_token", &"[redacted]")
            .finish()
    }
}

impl AuthUser {
    pub fn to_user(&self) -> User {
        User {
            id: self.id,
            username: self.username.clone(),
            email: self.email.clone(),
            permissions: db::model::user::UserPermissions::from_bits_truncate(
                self.permissions.try_into().unwrap_or(u32::MAX),
            ),
            password_hash: String::new(),
            last_sync: None,
            sync_token: None,
            sync_url: None,
            created_at: String::new(),
        }
    }

    #[cfg(test)]
    pub fn for_test(
        id: i64,
        username: &str,
        email: &str,
        permissions: usize,
        validity_token: Vec<u8>,
    ) -> Self {
        Self {
            id,
            username: username.to_string(),
            email: email.to_string(),
            permissions,
            validity_token,
        }
    }
}

impl AxumAuthUser for AuthUser {
    type Id = i64;

    fn id(&self) -> Self::Id {
        self.id
    }

    fn session_auth_hash(&self) -> &[u8] {
        &self.validity_token
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

    fn convert_user(user: User) -> AuthUser {
        AuthUser {
            id: user.id,
            username: user.username,
            email: user.email,
            permissions: user.permissions.bits() as usize,
            validity_token: user.sync_url.map_or_else(Vec::new, String::into_bytes),
        }
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

                Ok(user.map(Self::convert_user))
            }
            Credentials::Token(token_credentials) => {
                let user =
                    user_db::get_user_by_sync_token(&self.db, &token_credentials.token).await?;

                Ok(user.map(Self::convert_user))
            }
        }
    }

    async fn get_user(&self, user_id: &UserId<Self>) -> Result<Option<Self::User>, Self::Error> {
        let user = user_db::get_user_by_id(&self.db, *user_id).await?;

        Ok(user.map(Self::convert_user))
    }
}

// We use a type alias for convenience.
//
// Note that we've supplied our concrete backend here.
pub type AuthSession = axum_login::AuthSession<Backend>;

#[cfg(test)]
mod tests {
    use super::AuthUser;
    use db::model::user::UserPermissions;

    #[test]
    fn to_user_round_trip_id_username_email() {
        let auth = AuthUser::for_test(42, "alice", "alice@example.com", 0, vec![]);
        let user = auth.to_user();
        assert_eq!(user.id, 42);
        assert_eq!(user.username, "alice");
        assert_eq!(user.email, "alice@example.com");
        assert!(user.permissions.is_empty());
        assert!(user.password_hash.is_empty());
        assert!(user.sync_url.is_none());
    }

    #[test]
    fn to_user_permissions_bits() {
        let auth = AuthUser::for_test(
            1,
            "u",
            "e@e.com",
            UserPermissions::Admin.bits() as usize,
            vec![],
        );
        let user = auth.to_user();
        assert!(user.permissions.contains(UserPermissions::Admin));

        let auth_sync = AuthUser::for_test(
            2,
            "u",
            "e@e.com",
            UserPermissions::Sync.bits() as usize,
            vec![],
        );
        let user_sync = auth_sync.to_user();
        assert!(user_sync.permissions.contains(UserPermissions::Sync));
    }

    #[test]
    fn to_user_permissions_truncate_large_usize() {
        let auth = AuthUser::for_test(1, "u", "e@e.com", usize::MAX, vec![]);
        let user = auth.to_user();
        assert_eq!(
            user.permissions.bits(),
            UserPermissions::Admin.bits()
                | UserPermissions::Sync.bits()
                | UserPermissions::OnlineAccount.bits()
        );
    }
}
