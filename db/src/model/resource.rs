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
        let mut flags = Vec::new();
        if self.contains(Self::Completed) {
            flags.push("Completed");
        }
        flags.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ResourceFlags {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let flags: Vec<String> = Vec::deserialize(deserializer)?;
        let mut result = Self::empty();
        for flag in flags {
            if flag.as_str() == "Completed" {
                result.insert(Self::Completed);
            }
        }
        Ok(result)
    }
}

#[derive(Serialize, Clone)]
pub struct ResourceMeta {
    pub id: i64,
    pub name: String,
    pub flags: ResourceFlags,
    pub created: String,
    pub last_modified: String,
    pub unique_global_id: String,
}

impl ResourceMeta {
    /// Builds a `ResourceMeta` from raw DB columns, centralizing the `resource_flags`
    /// (stored as i64) -> `ResourceFlags` conversion shared by every resource query.
    #[must_use]
    pub fn from_parts(
        id: i64,
        name: String,
        flags: i64,
        created: String,
        unique_global_id: String,
        last_modified: String,
    ) -> Self {
        Self {
            id,
            name,
            flags: ResourceFlags::from_bits(u32::try_from(flags).unwrap_or(0))
                .unwrap_or(ResourceFlags::empty()),
            created,
            unique_global_id,
            last_modified,
        }
    }
}

#[derive(Serialize)]
pub struct Resource {
    pub meta: ResourceMeta,
    pub groups: Vec<ModelGroup>,
}
