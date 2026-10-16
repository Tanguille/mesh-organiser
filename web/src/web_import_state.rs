use service::import_state::{ImportState, ImportStateEmitter};

pub struct WebImportStateEmitter;

impl ImportStateEmitter for WebImportStateEmitter {
    fn status_event(&self, status: &ImportState) {
        println!("Import Status: {}", status.status);
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
}
