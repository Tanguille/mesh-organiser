use std::{num::NonZeroUsize, thread};

use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;

use crate::slicer_service::Slicer;

/// Returns `None` for the `slicer` field when it is absent from a settings file.
///
/// The container-level `#[serde(default)]` would otherwise fall back to
/// `Configuration::default().slicer`, which runs the side-effecting
/// `is_installed()` / flatpak auto-detect. A settings file missing the slicer
/// key must stay `None` (the historical behaviour), so we override the default.
const fn default_slicer() -> Option<Slicer> {
    Option::<Slicer>::None
}

/// Application configuration. Many boolean flags for UI/slicer behaviour.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Configuration {
    pub data_path: String,
    pub prusa_deep_link: bool,
    pub cura_deep_link: bool,
    pub bambu_deep_link: bool,
    pub orca_deep_link: bool,
    pub open_slicer_on_remote_model_import: bool,
    pub show_ungrouped_models_in_groups: bool,
    #[serde(default = "default_slicer")]
    pub slicer: Option<Slicer>,
    pub focus_after_link_import: bool,
    pub thumbnail_color: String,
    pub size_option_models: String,
    pub size_option_groups: String,
    pub show_grouped_count_on_labels: bool,
    pub fallback_3mf_thumbnail: bool,
    pub prefer_3mf_thumbnail: bool,
    pub core_parallelism: usize,
    pub collapse_sidebar: bool,
    pub zoom_level: u32,
    pub export_metadata: bool,
    pub show_date_on_list_view: bool,
    pub default_enabled_recursive_import: bool,
    pub default_enabled_delete_after_import: bool,
    pub open_links_in_external_browser: bool,
    pub max_size_model_3mf_preview: u32,
    pub max_size_model_stl_preview: u32,
    pub max_size_model_obj_preview: u32,
    pub max_size_model_step_preview: u32,
    pub only_show_single_image_in_groups: bool,
    pub custom_slicer_path: String,
    pub elegoo_deep_link: bool,
    pub group_split_view: String,
    pub label_exported_model_as_printed: bool,
    pub theme: String,
    pub order_option_models: String,
    pub order_option_groups: String,
    pub ignore_update: String,
    pub show_multiselect_checkboxes: bool,
    pub use_worker_for_model_parsing: bool,
    pub prefer_gcode_thumbnail: bool,
    pub last_user_id: i64,
    pub custom_css: String,
    pub default_enabled_import_as_path: bool,
    pub thumbnail_rotation: [i16; 3],
    pub watch_downloads_folder: bool,
    pub startup_page: String,
}

impl Default for Configuration {
    fn default() -> Self {
        let installed_slicer = Slicer::iter().find(Slicer::is_installed);
        let mut parallelism = thread::available_parallelism()
            .unwrap_or(NonZeroUsize::new(3).unwrap())
            .get()
            / 2;

        if parallelism == 0 {
            parallelism = 1;
        }

        Self {
            data_path: String::new(),
            prusa_deep_link: false,
            cura_deep_link: false,
            bambu_deep_link: false,
            orca_deep_link: false,
            open_slicer_on_remote_model_import: false,
            show_ungrouped_models_in_groups: true,
            slicer: installed_slicer,
            focus_after_link_import: true,
            thumbnail_color: String::from("#EEEEEE"),
            size_option_models: String::from("Grid_Medium"),
            size_option_groups: String::from("Grid_Medium"),
            show_grouped_count_on_labels: true,
            fallback_3mf_thumbnail: true,
            prefer_3mf_thumbnail: true,
            core_parallelism: parallelism,
            collapse_sidebar: false,
            zoom_level: 100,
            export_metadata: false,
            show_date_on_list_view: false,
            default_enabled_recursive_import: false,
            default_enabled_delete_after_import: false,
            open_links_in_external_browser: true,
            max_size_model_3mf_preview: 15,
            max_size_model_stl_preview: 30,
            max_size_model_obj_preview: 30,
            max_size_model_step_preview: 10,
            only_show_single_image_in_groups: true,
            custom_slicer_path: String::new(),
            elegoo_deep_link: false,
            group_split_view: String::from("split-left-right"),
            label_exported_model_as_printed: false,
            theme: String::from("default"),
            order_option_models: String::from("modified-desc"),
            order_option_groups: String::from("modified-desc"),
            ignore_update: String::new(),
            show_multiselect_checkboxes: true,
            use_worker_for_model_parsing: true,
            prefer_gcode_thumbnail: true,
            last_user_id: 1,
            custom_css: String::new(),
            default_enabled_import_as_path: false,
            thumbnail_rotation: [35, 30, 0],
            watch_downloads_folder: false,
            startup_page: String::new(),
        }
    }
}

// -----------------------------------------------------------------------------
// Regression tests: lock in Configuration::default() and the serde deserialization
// path that replaced stored_to_configuration, so defaults and the slicer-stays-None
// behaviour for missing/null keys don't regress.
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_default_has_expected_values() {
        let config = Configuration::default();
        assert_eq!(config.data_path, "");
        assert_eq!(config.thumbnail_rotation, [35, 30, 0]);
        assert!(
            config.core_parallelism >= 1,
            "core_parallelism must be at least 1"
        );
        assert_eq!(config.theme, "default");
        assert_eq!(config.last_user_id, 1);
        assert!(!config.export_metadata);
        assert!(config.fallback_3mf_thumbnail);
        assert!(config.prefer_3mf_thumbnail);
    }

    // (a) A settings file with no slicer key must stay None, not run the
    // side-effecting is_installed() auto-detect from Configuration::default().
    #[test]
    fn deserialize_missing_slicer_key_is_none() {
        let json = r#"{ "data_path": "/custom/data" }"#;
        let config: Configuration = serde_json::from_str(json).expect("deserialize");
        assert!(config.slicer.is_none());
    }

    // (b) An explicit null slicer also stays None.
    #[test]
    fn deserialize_null_slicer_is_none() {
        let json = r#"{ "slicer": null }"#;
        let config: Configuration = serde_json::from_str(json).expect("deserialize");
        assert!(config.slicer.is_none());
    }

    // (c) A present slicer deserializes to that variant.
    #[test]
    fn deserialize_present_slicer_is_some_variant() {
        let json = r#"{ "slicer": "PrusaSlicer" }"#;
        let config: Configuration = serde_json::from_str(json).expect("deserialize");
        assert!(matches!(config.slicer, Some(Slicer::PrusaSlicer)));
    }

    // (d) An empty object falls back to Configuration::default() for every
    // non-slicer field (the old stored_to_configuration unwrap_or(default) path).
    #[test]
    fn deserialize_empty_object_uses_defaults() {
        let config: Configuration = serde_json::from_str("{}").expect("deserialize");
        let default = Configuration::default();
        assert_eq!(config.data_path, default.data_path);
        assert_eq!(config.theme, default.theme);
        assert_eq!(config.thumbnail_rotation, default.thumbnail_rotation);
        assert_eq!(config.core_parallelism, default.core_parallelism);
        assert_eq!(config.last_user_id, default.last_user_id);
        assert_eq!(config.zoom_level, default.zoom_level);
        assert_eq!(config.export_metadata, default.export_metadata);
        assert_eq!(
            config.fallback_3mf_thumbnail,
            default.fallback_3mf_thumbnail
        );
        assert_eq!(config.prefer_3mf_thumbnail, default.prefer_3mf_thumbnail);
        assert_eq!(
            config.show_ungrouped_models_in_groups,
            default.show_ungrouped_models_in_groups
        );
        assert_eq!(config.group_split_view, default.group_split_view);
    }

    // Rewritten from the former stored_to_configuration_empty_uses_defaults:
    // deserialize JSON straight into Configuration instead of building a literal.
    #[test]
    fn deserialize_empty_uses_defaults() {
        let config: Configuration = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(config.data_path, Configuration::default().data_path);
        assert_eq!(config.theme, "default");
        assert_eq!(config.thumbnail_rotation, [35, 30, 0]);
    }

    // Rewritten from the former stored_to_configuration_overrides_single_field:
    // a single overridden field wins; everything else falls back to default.
    #[test]
    fn deserialize_overrides_single_field() {
        let default = Configuration::default();
        let json = r#"{ "data_path": "/custom/data" }"#;
        let config: Configuration = serde_json::from_str(json).expect("deserialize");
        assert_eq!(config.data_path, "/custom/data");
        assert_eq!(config.theme, default.theme);
    }
}
