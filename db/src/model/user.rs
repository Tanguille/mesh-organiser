use bitflags::bitflags;
use password_auth::generate_hash;
use serde::{Deserialize, Serialize};

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct UserPermissions: u32 {
        const Admin = 0b0000_0001;
        const Sync  = 0b0000_0010;
        const OnlineAccount = 0b0000_0100;
    }
}

impl Serialize for UserPermissions {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.iter_names()
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for UserPermissions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Vec::<String>::deserialize(deserializer)?
            .iter()
            .filter_map(|f| Self::from_name(f))
            .collect())
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub created_at: String,
    pub sync_url: Option<String>,
    pub sync_token: Option<String>,
    pub last_sync: Option<String>,
    pub permissions: UserPermissions,
    #[serde(skip_serializing)]
    pub password_hash: String,
}

impl Default for User {
    fn default() -> Self {
        Self {
            id: 1,
            username: String::default(),
            email: String::default(),
            created_at: String::default(),
            sync_url: None,
            sync_token: None,
            last_sync: None,
            password_hash: String::default(),
            permissions: UserPermissions::empty(),
        }
    }
}

#[must_use]
pub fn hash_password(password: &str) -> String {
    generate_hash(password)
}
