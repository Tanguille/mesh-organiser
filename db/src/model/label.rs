use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct LabelMeta {
    pub id: i64,
    pub name: String,
    pub color: i64,
    pub unique_global_id: String,
    pub last_modified: String,
}

#[derive(Serialize, Debug)]
pub struct Label {
    pub meta: LabelMeta,
    pub children: Vec<LabelMeta>,
    pub effective_labels: Vec<LabelMeta>,
    pub has_parent: bool,
    pub model_count: i64,
    pub group_count: i64,
    pub self_model_count: i64,
    pub self_group_count: i64,
}
