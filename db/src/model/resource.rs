use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use super::model_group::ModelGroup;

bitflags! {
    #[derive(Clone, Copy)]
    pub struct ResourceFlags: u32 {
        const Completed  = 0b0000_0001;
    }
}

impl Serialize for ResourceFlags {
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

impl<'de> Deserialize<'de> for ResourceFlags {
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

/// Decodes the stored `resource_flags` column; a value with unknown bits decodes as no flags.
impl From<i64> for ResourceFlags {
    fn from(bits: i64) -> Self {
        Self::from_bits(u32::try_from(bits).unwrap_or(0)).unwrap_or_else(Self::empty)
    }
}

/// Maps from the `resources` columns by name, so it also reads a resource out of a joined row.
#[derive(Serialize, Clone, sqlx::FromRow)]
pub struct ResourceMeta {
    #[sqlx(rename = "resource_id")]
    pub id: i64,
    #[sqlx(rename = "resource_name")]
    pub name: String,
    #[sqlx(rename = "resource_flags", try_from = "i64")]
    pub flags: ResourceFlags,
    #[sqlx(rename = "resource_created")]
    pub created: String,
    #[sqlx(rename = "resource_last_modified")]
    pub last_modified: String,
    #[sqlx(rename = "resource_unique_global_id")]
    pub unique_global_id: String,
}

#[derive(Serialize)]
pub struct Resource {
    pub meta: ResourceMeta,
    pub groups: Vec<ModelGroup>,
}
