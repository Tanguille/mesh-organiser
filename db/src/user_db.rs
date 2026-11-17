use rand::RngExt;

use crate::{
    DbError,
    db_context::DbContext,
    model::user::{User, UserPermissions, hash_password},
    random_hex_32,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct SyncIdentity {
    pub instance_id: String,
    pub account_id: String,
}

pub async fn get_sync_identity(
    db: &DbContext,
    user_id: i64,
) -> Result<Option<SyncIdentity>, DbError> {
    sqlx::query_as(
        "SELECT sync_instance.instance_id, sync_accounts.account_id FROM sync_instance JOIN sync_accounts ON sync_accounts.user_id = ? WHERE sync_instance.singleton = 1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
}

#[derive(sqlx::FromRow)]
#[allow(clippy::struct_field_names)]
struct UserDbQuery {
    user_id: i64,
    user_name: String,
    user_email: String,
    user_created_at: String,
    user_last_sync: Option<String>,
    user_sync_token: Option<String>,
    user_sync_url: Option<String>,
    user_permissions: i64,
    user_password_hash: String,
}

#[derive(sqlx::FromRow)]
struct AuthUserDbQuery {
    #[sqlx(flatten)]
    user: UserDbQuery,
    account_id: String,
}

#[derive(Clone, Copy)]
pub enum AuthUserLookup<'a> {
    Id(i64),
    Email(&'a str),
    SyncToken(&'a str),
}

pub async fn get_auth_user(
    db: &DbContext,
    lookup: AuthUserLookup<'_>,
) -> Result<Option<(User, String)>, DbError> {
    let mut query = sqlx::QueryBuilder::new(
        "SELECT u.user_id, u.user_name, u.user_email, u.user_created_at, u.user_last_sync, u.user_sync_token, u.user_sync_url, u.user_permissions, u.user_password_hash, a.account_id FROM users u JOIN sync_accounts a ON a.user_id = u.user_id WHERE ",
    );
    match lookup {
        AuthUserLookup::Id(id) => {
            query.push("u.user_id = ").push_bind(id);
        }
        AuthUserLookup::Email(email) => {
            query.push("u.user_email = ").push_bind(email);
        }
        AuthUserLookup::SyncToken(token) => {
            query.push("u.user_sync_token = ").push_bind(token);
        }
    }

    Ok(query
        .build_query_as::<AuthUserDbQuery>()
        .fetch_optional(db)
        .await?
        .map(|row| (row.user.into_user(), row.account_id)))
}

impl UserDbQuery {
    fn into_user(self) -> User {
        User {
            id: self.user_id,
            username: self.user_name,
            email: self.user_email,
            created_at: self.user_created_at,
            last_sync: self.user_last_sync,
            sync_token: self.user_sync_token,
            sync_url: self.user_sync_url,
            permissions: UserPermissions::from_bits_truncate(
                u32::try_from(self.user_permissions).unwrap_or(0),
            ),
            password_hash: self.user_password_hash,
        }
    }
}

pub async fn get_users(db: &DbContext) -> Result<Vec<User>, DbError> {
    let rows = sqlx::query_as!(
        UserDbQuery,
        "SELECT user_id, 
            user_name, 
            user_email, 
            user_created_at, 
            user_last_sync, 
            user_sync_token,
            user_sync_url,
            user_permissions,
            user_password_hash FROM users"
    )
    .fetch_all(db)
    .await?;

    Ok(rows.into_iter().map(UserDbQuery::into_user).collect())
}

pub async fn get_user_by_id(db: &DbContext, user_id: i64) -> Result<Option<User>, DbError> {
    let row = sqlx::query_as!(
        UserDbQuery,
        "SELECT user_id, 
            user_name, 
            user_email, 
            user_created_at, 
            user_last_sync, 
            user_sync_token,
            user_sync_url,
            user_permissions,
            user_password_hash FROM users WHERE user_id = ?",
        user_id
    )
    .fetch_optional(db)
    .await?;

    Ok(row.map(UserDbQuery::into_user))
}

pub async fn add_user(
    db: &DbContext,
    username: &str,
    email: &str,
    password: &str,
) -> Result<i64, DbError> {
    let now = crate::time_now();
    let id = i64::from(rand::rng().random::<u32>());
    let password = hash_password(password);

    let result = sqlx::query!(
        "INSERT INTO users (user_id, user_name, user_email, user_password_hash, user_created_at) VALUES (?, ?, ?, ?, ?)",
        id,
        username,
        email,
        password,
        now
    )
    .execute(db)
    .await?;

    let user_id = result.last_insert_rowid();

    Ok(user_id)
}

pub async fn edit_user_min(
    db: &DbContext,
    user_id: i64,
    username: &str,
    email: &str,
) -> Result<(), DbError> {
    sqlx::query!(
        "UPDATE users SET user_name = ?, user_email = ? WHERE user_id = ?",
        username,
        email,
        user_id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn edit_user_last_sync_time(
    db: &DbContext,
    user_id: i64,
    user_last_sync: &str,
) -> Result<(), DbError> {
    sqlx::query!(
        "UPDATE users SET user_last_sync = ? WHERE user_id = ?",
        user_last_sync,
        user_id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn set_user_sync_token(
    db: &DbContext,
    user_id: i64,
    sync_token: &str,
    sync_url: &str,
    online: bool,
) -> Result<(), DbError> {
    let clear_bits = i64::from(UserPermissions::OnlineAccount.bits() ^ u32::MAX);
    let set_bits = if online {
        i64::from(UserPermissions::OnlineAccount.bits())
    } else {
        0
    };

    sqlx::query!(
        "UPDATE users SET user_sync_token = ?, user_sync_url = ?, user_last_sync = NULL, user_permissions = (user_permissions & ?) | ? WHERE user_id = ?",
        sync_token,
        sync_url,
        clear_bits,
        set_bits,
        user_id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn clear_user_sync(db: &DbContext, user_id: i64) -> Result<(), DbError> {
    let mut transaction = db.begin().await?;
    let bits = i64::from(UserPermissions::OnlineAccount.bits() ^ u32::MAX);

    sqlx::query!(
        "UPDATE users SET user_sync_token = NULL, user_sync_url = NULL, user_permissions = user_permissions & ? WHERE user_id = ?",
        bits,
        user_id
    )
    .execute(&mut *transaction)
    .await?;

    sqlx::query("DELETE FROM sync_transport_local_authorities WHERE user_id=?")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM causal_account_writers WHERE user_id=?")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;

    Ok(())
}

pub async fn scramble_validity_token(db: &DbContext, user_id: i64) -> Result<(), DbError> {
    let random = random_hex_32();

    sqlx::query!(
        "UPDATE users SET user_sync_url = ? WHERE user_id = ?",
        random,
        user_id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn scramble_login_token(db: &DbContext, user_id: i64) -> Result<(), DbError> {
    let random = random_hex_32();

    sqlx::query!(
        "UPDATE users SET user_sync_token = ? WHERE user_id = ?",
        random,
        user_id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn edit_user_password(
    db: &DbContext,
    user_id: i64,
    password: &str,
) -> Result<(), DbError> {
    let password = hash_password(password);
    sqlx::query!(
        "UPDATE users SET user_password_hash = ? WHERE user_id = ?",
        password,
        user_id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn set_user_permissions(
    db: &DbContext,
    user_id: i64,
    permissions: UserPermissions,
) -> Result<(), DbError> {
    let bits = i64::from(permissions.bits());
    sqlx::query!(
        "UPDATE users SET user_permissions = ? WHERE user_id = ?",
        bits,
        user_id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn delete_user(db: &DbContext, user_id: i64) -> Result<(), DbError> {
    sqlx::query!("DELETE FROM users WHERE user_id = ?", user_id)
        .execute(db)
        .await?;

    Ok(())
}

pub async fn get_user_by_email(db: &DbContext, email: &str) -> Result<Option<User>, DbError> {
    let row = sqlx::query_as!(
        UserDbQuery,
        "SELECT user_id, 
            user_name, 
            user_email, 
            user_created_at, 
            user_last_sync, 
            user_sync_token,
            user_sync_url,
            user_permissions,
            user_password_hash FROM users WHERE user_email = ?",
        email
    )
    .fetch_optional(db)
    .await?;

    Ok(row.map(UserDbQuery::into_user))
}

pub async fn get_user_by_sync_token(
    db: &DbContext,
    sync_token: &str,
) -> Result<Option<User>, DbError> {
    let row = sqlx::query_as!(
        UserDbQuery,
        "SELECT user_id, 
            user_name, 
            user_email, 
            user_created_at, 
            user_last_sync, 
            user_sync_token,
            user_sync_url,
            user_permissions,
            user_password_hash FROM users WHERE user_sync_token = ?",
        sync_token
    )
    .fetch_optional(db)
    .await?;

    Ok(row.map(UserDbQuery::into_user))
}
