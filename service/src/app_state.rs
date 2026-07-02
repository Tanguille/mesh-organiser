use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use db::db_context::DbContext;

use crate::configuration::Configuration;

pub struct AppState {
    pub db: Arc<DbContext>,
    pub configuration: Mutex<Configuration>,
    pub import_mutex: Arc<tokio::sync::Mutex<()>>,
    pub app_data_path: String,
}

impl AppState {
    /// Pushes `name` onto `base`, creating the directory if it does not exist, and returns it.
    ///
    /// # Panics
    ///
    /// Panics if the directory cannot be created (e.g. permission or I/O error).
    fn ensure_dir(base: PathBuf, name: &str) -> PathBuf {
        let mut path_buff = base;
        path_buff.push(name);

        if !path_buff.exists() {
            fs::create_dir_all(&path_buff)
                .unwrap_or_else(|e| panic!("Failed to create {name} directory: {e}"));
        }

        path_buff
    }

    /// Returns the models directory path, creating it if it does not exist.
    ///
    /// # Panics
    ///
    /// Panics if the model directory cannot be created (e.g. permission or I/O error).
    pub fn get_model_dir(&self) -> PathBuf {
        Self::ensure_dir(PathBuf::from(self.get_configuration().data_path), "models")
    }

    /// Returns the images directory path, creating it if it does not exist.
    ///
    /// # Panics
    ///
    /// Panics if the image directory cannot be created (e.g. permission or I/O error).
    pub fn get_image_dir(&self) -> PathBuf {
        Self::ensure_dir(PathBuf::from(&self.app_data_path), "images")
    }

    /// Returns the resources directory path, creating it if it does not exist.
    ///
    /// # Panics
    ///
    /// Panics if the resources directory cannot be created (e.g. permission or I/O error).
    pub fn get_resources_dir(&self) -> PathBuf {
        Self::ensure_dir(
            PathBuf::from(self.get_configuration().data_path),
            "resources",
        )
    }

    /// Returns a copy of the current configuration.
    ///
    /// # Panics
    ///
    /// Panics if the configuration mutex is poisoned.
    pub fn get_configuration(&self) -> Configuration {
        self.configuration.lock().unwrap().clone()
    }
}

impl Clone for AppState {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            configuration: Mutex::new(self.get_configuration()),
            app_data_path: self.app_data_path.clone(),
            import_mutex: Arc::clone(&self.import_mutex),
        }
    }
}
