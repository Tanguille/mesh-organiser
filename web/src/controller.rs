use serde::Deserialize;

pub mod auth_controller;
pub mod blob_controller;
pub mod group_controller;
pub mod label_controller;
pub mod model_controller;
pub mod page_controller;
pub mod resource_controller;
pub mod share_controller;
pub mod threemf_controller;
pub mod user_controller;

/// Request body shared by every endpoint that takes just a list of model ids.
#[derive(Deserialize)]
pub struct ModelIdsParams {
    pub model_ids: Vec<i64>,
}
