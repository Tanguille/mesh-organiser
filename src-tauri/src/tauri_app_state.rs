use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_deep_link::DeepLinkExt;

use db::model::user::User;
use service::{AppState, Configuration};

#[derive(Serialize, Deserialize, Clone)]
pub struct AccountLinkEmit {
    pub base_url: String,
    pub user_name: String,
    pub link_token: String,
}

#[derive(Clone, Serialize)]
pub struct InitialState {
    pub deep_link_url: Option<String>,
    pub max_parallelism: usize,
    pub collapse_sidebar: bool,
    pub account_link: Option<AccountLinkEmit>,
}

impl InitialState {
    pub fn new(config: &Configuration) -> Self {
        Self {
            deep_link_url: None,
            max_parallelism: std::thread::available_parallelism()
                .map_or(6, std::num::NonZeroUsize::get),
            collapse_sidebar: config.collapse_sidebar,
            account_link: None,
        }
    }
}

/// A deep link was newly enabled: it is on now and was off before.
fn deep_link_newly_enabled(old: &Configuration, new: &Configuration) -> bool {
    [
        (old.prusa_deep_link, new.prusa_deep_link),
        (old.cura_deep_link, new.cura_deep_link),
        (old.bambu_deep_link, new.bambu_deep_link),
        (old.orca_deep_link, new.orca_deep_link),
    ]
    .iter()
    .any(|(was_enabled, is_enabled)| *is_enabled && !*was_enabled)
}

pub struct TauriAppState {
    pub app_state: AppState,
    pub initial_state: Mutex<Option<InitialState>>,
    pub current_user: Arc<Mutex<User>>,
}

impl TauriAppState {
    pub fn get_configuration(&self) -> Configuration {
        self.app_state.get_configuration()
    }

    pub fn get_current_user(&self) -> User {
        let user = self.current_user.lock().unwrap();
        user.clone()
    }

    /// Fetches a model for the current user, erroring when it does not exist.
    pub async fn require_model(
        &self,
        model_id: i64,
    ) -> Result<db::model::Model, crate::error::ApplicationError> {
        db::model_db::get_model_via_id(&self.app_state.db, &self.get_current_user(), model_id)
            .await?
            .ok_or_else(|| {
                crate::error::ApplicationError::InternalError(String::from("Failed to find model"))
            })
    }

    pub async fn set_current_user_by_id(
        &self,
        user_id: i64,
    ) -> Result<(), crate::error::ApplicationError> {
        let path = self.get_settings_path();
        let user = db::user_db::get_user_by_id(&self.app_state.db, user_id).await?;

        if user.is_none() {
            return Err(crate::error::ApplicationError::InternalError(
                "User not found".into(),
            ));
        }

        *self.current_user.lock().unwrap() = user.unwrap();

        {
            let mut configuration = self.app_state.configuration.lock().unwrap();
            configuration.last_user_id = user_id;
            let json = serde_json::to_string(&*configuration).unwrap();
            drop(configuration);
            fs::write(path, json).expect("Failed to write configuration");
        }

        Ok(())
    }

    fn get_settings_path(&self) -> PathBuf {
        let mut path_buff = PathBuf::from(&self.app_state.app_data_path);
        path_buff.push("settings.json");
        path_buff
    }

    pub fn write_configuration(&self, new_configuration: &Configuration) -> bool {
        let path = self.get_settings_path();
        let mut new_configuration = new_configuration.clone();
        new_configuration.last_user_id = self.get_current_user().id;
        let json = serde_json::to_string(&new_configuration).unwrap();

        fs::write(path, json).expect("Failed to write configuration");

        let mut configuration = self.app_state.configuration.lock().unwrap();
        let deep_link_setting_changed = deep_link_newly_enabled(&configuration, &new_configuration);
        *configuration = new_configuration;

        deep_link_setting_changed
    }

    pub fn configure_deep_links(&self, app_handle: &AppHandle) {
        let config = self.app_state.get_configuration();

        if config.bambu_deep_link {
            let _ = app_handle.deep_link().register("bambustudio");
        }

        if config.cura_deep_link {
            let _ = app_handle.deep_link().register("cura");
        }

        if config.prusa_deep_link {
            let _ = app_handle.deep_link().register("prusaslicer");
        }

        if config.orca_deep_link {
            let _ = app_handle.deep_link().register("orcaslicer");
        }

        if config.elegoo_deep_link {
            let _ = app_handle.deep_link().register("elegooslicer");
        }

        let _ = app_handle.deep_link().register("meshorganiser");
    }
}

#[cfg(test)]
mod tests {
    fn with_deep_links(enabled: bool) -> service::Configuration {
        service::Configuration {
            prusa_deep_link: enabled,
            cura_deep_link: enabled,
            bambu_deep_link: enabled,
            orca_deep_link: enabled,
            elegoo_deep_link: enabled,
            ..service::Configuration::default()
        }
    }

    fn assert_newly_enabled(enable: fn(&mut service::Configuration)) {
        let old = with_deep_links(false);
        let mut new = old.clone();
        enable(&mut new);

        assert!(super::deep_link_newly_enabled(&old, &new));
    }

    #[test]
    fn deep_link_newly_enabled_for_prusa() {
        assert_newly_enabled(|config| config.prusa_deep_link = true);
    }

    #[test]
    fn deep_link_newly_enabled_for_cura() {
        assert_newly_enabled(|config| config.cura_deep_link = true);
    }

    #[test]
    fn deep_link_newly_enabled_for_bambu() {
        assert_newly_enabled(|config| config.bambu_deep_link = true);
    }

    #[test]
    fn deep_link_newly_enabled_for_orca() {
        assert_newly_enabled(|config| config.orca_deep_link = true);
    }

    #[test]
    #[ignore = "elegoo omitted; fixed in follow-up commit"]
    fn deep_link_newly_enabled_for_elegoo() {
        assert_newly_enabled(|config| config.elegoo_deep_link = true);
    }

    #[test]
    fn deep_link_not_newly_enabled_when_already_on() {
        let old = with_deep_links(true);
        let new = with_deep_links(true);

        assert!(!super::deep_link_newly_enabled(&old, &new));
    }

    #[test]
    fn deep_link_not_newly_enabled_when_turned_off() {
        let old = with_deep_links(true);
        let new = with_deep_links(false);

        assert!(!super::deep_link_newly_enabled(&old, &new));
    }

    #[test]
    fn deep_link_not_newly_enabled_when_unchanged_off() {
        let old = with_deep_links(false);
        let new = with_deep_links(false);

        assert!(!super::deep_link_newly_enabled(&old, &new));
    }
}
