use bitflags::bitflags;
use serde::{Deserialize, Serialize};

pub mod blob;
pub mod label;
pub mod label_keyword;
pub mod model_group;
pub mod resource;
pub mod share;
pub mod user;

bitflags! {
    #[derive(Debug, Default)]
    pub struct ModelFlags: u32 {
        const Printed  = 0b0000_0001;
        const Favorite = 0b0000_0010;
    }
}

impl Serialize for ModelFlags {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut flags = Vec::new();
        if self.contains(Self::Printed) {
            flags.push("Printed");
        }
        if self.contains(Self::Favorite) {
            flags.push("Favorite");
        }
        flags.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ModelFlags {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let flags: Vec<String> = Vec::deserialize(deserializer)?;
        let mut result = Self::empty();
        for flag in flags {
            match flag.as_str() {
                "Printed" => result.insert(Self::Printed),
                "Favorite" => result.insert(Self::Favorite),
                _ => {}
            }
        }
        Ok(result)
    }
}

// TODO: Change this model entirely. Discard model_user. Return group id back to model. Back this instead by a blob table.
#[derive(Serialize)]
pub struct Model {
    pub id: i64,
    pub name: String,
    pub blob: blob::Blob,
    pub link: Option<String>,
    pub description: Option<String>,
    pub added: String,
    pub last_modified: String,
    pub group: Option<model_group::ModelGroupMeta>,
    pub labels: Vec<label::LabelMeta>,
    pub flags: ModelFlags,
    pub unique_global_id: String,
}
