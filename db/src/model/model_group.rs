use serde::Serialize;

use super::{Model, ModelFlags, label::LabelMeta, resource::ResourceMeta};

#[derive(Serialize, Clone)]
pub struct ModelGroupMeta {
    pub id: i64,
    pub name: String,
    pub created: String,
    pub last_modified: String,
    pub resource_id: Option<i64>,
    pub unique_global_id: String,
}

#[derive(Serialize)]
pub struct ModelGroup {
    pub meta: ModelGroupMeta,
    pub models: Vec<Model>,
    pub labels: Vec<LabelMeta>,
    pub resource: Option<ResourceMeta>,
    pub flags: ModelFlags,
}

impl ModelGroup {
    #[must_use]
    pub const fn from_meta(meta: ModelGroupMeta) -> Self {
        Self {
            meta,
            models: Vec::new(),
            labels: Vec::new(),
            resource: None,
            flags: ModelFlags::empty(),
        }
    }
}

// NOTE (2026-04-15): Group-level labels are aggregated when building `ModelGroup` in `group_db::convert_model_list_to_groups`
// (union of member models’ labels). No `ModelGroup` helper is referenced from `service/` or `src-tauri/`; revisit only if product
// rules need richer “effective” labels (e.g. inherited defaults) on this type.
