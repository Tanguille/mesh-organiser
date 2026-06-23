use db::{
    model::{Model, blob::Blob, user::User},
    model_db,
};
use service::{
    import_state::{ImportState, ImportStateEmitter, ImportStatus},
    thumbnail_service,
};

use crate::{error::ApplicationError, web_app_state::WebAppState};

/// Fetches the given models and generates their thumbnails into `import_state`.
/// Shared by the model-upload and 3MF-extract handlers, which both run this same tail.
pub async fn generate_thumbnails_for_models(
    app_state: &WebAppState,
    user: &User,
    model_ids: &[i64],
    import_state: &mut ImportState,
) -> Result<Vec<Model>, ApplicationError> {
    let models =
        model_db::get_models_via_ids(&app_state.app_state.db, user, model_ids.to_vec()).await?;
    let blobs: Vec<&Blob> = models.iter().map(|m| &m.blob).collect();

    thumbnail_service::generate_thumbnails(&blobs, &app_state.app_state, false, import_state)
        .await?;

    Ok(models)
}

pub struct WebImportStateEmitter;

impl ImportStateEmitter for WebImportStateEmitter {
    fn status_event(&self, status: &ImportState) {
        match status.status {
            ImportStatus::ProcessingThumbnails => println!("Import Status: Processing Thumbnails"),
            ImportStatus::Finished => println!("Import Status: Finished"),
            ImportStatus::Failure => println!("Import Status: Failure"),
            ImportStatus::FinishedModels => println!("Import Status: Finished Models"),
            ImportStatus::ProcessingModels => println!("Import Status: Processing Models"),
            ImportStatus::Idle => println!("Import Status: Idle"),
            ImportStatus::FinishedThumbnails => println!("Import Status: Finished Thumbnails"),
        }
    }

    fn model_total_event(&self, status: &ImportState) {
        if status.model_count == 0 {
            return;
        }

        println!("Preparing to import {} models", status.model_count);
    }

    fn failure_reason_event(&self, status: &ImportState) {
        if let Some(reason) = &status.failure_reason {
            println!("Import Failure: {reason}");
        }
    }

    fn model_group_event(&self, status: &ImportState) {
        if let Some(group_name) = status.get_last_group_name() {
            println!("Importing Group '{group_name}'");
        }
    }

    fn thumbnail_count_event(&self, status: &ImportState) {
        if status.model_count == 0 && status.finished_thumbnails_count == 0 {
            return;
        }

        println!(
            "Processed {}/{} thumbnails",
            status.finished_thumbnails_count, status.model_count
        );
    }

    fn model_count_event(&self, status: &ImportState) {
        if status.model_count == 0 && status.imported_model_count == 0 {
            return;
        }

        println!(
            "Imported {}/{} models",
            status.imported_model_count, status.model_count
        );
    }

    fn all_data_event(&self, _state: &ImportState) {}
}
