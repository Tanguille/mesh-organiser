use std::str::FromStr;

use tauri::State;

use db::{
    group_db::{self, GroupOrderBy},
    model::model_group::{ModelGroup, ModelGroupMeta},
};

use crate::{
    error::ApplicationError, mobile_guard::require_local_desktop_app,
    tauri_app_state::TauriAppState,
};

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn get_groups(
    model_ids: Option<Vec<i64>>,
    group_ids: Option<Vec<i64>>,
    label_ids: Option<Vec<i64>>,
    order_by: Option<String>,
    text_search: Option<String>,
    page: u32,
    page_size: u32,
    include_ungrouped_models: Option<bool>,
    state: State<'_, TauriAppState>,
) -> Result<Vec<ModelGroup>, ApplicationError> {
    require_local_desktop_app()?;

    let groups = group_db::get_groups(
        &state.app_state.db,
        &state.get_current_user(),
        group_db::GroupFilterOptions {
            model_ids,
            group_ids,
            label_ids,
            order_by: order_by
                .map(|s| GroupOrderBy::from_str(&s))
                .transpose()
                .map_err(|_| ApplicationError::InternalError(
                    "Invalid order_by value. Valid values are: CreatedAsc, CreatedDesc, NameAsc, NameDesc, ModifiedAsc, ModifiedDesc".to_string()
                ))?,
            text_search,
            page,
            page_size,
            include_ungrouped_models: include_ungrouped_models.unwrap_or(false),
            allow_incomplete_groups: false,
            split_incomplete_groups: false,
        },
    )
    .await?;

    Ok(groups.items)
}

#[tauri::command]
pub async fn ungroup(
    group_id: i64,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    require_local_desktop_app()?;

    group_db::delete_group(&state.app_state.db, &state.get_current_user(), group_id).await?;

    Ok(())
}

#[tauri::command]
pub async fn add_group(
    group_name: &str,
    state: State<'_, TauriAppState>,
) -> Result<ModelGroupMeta, ApplicationError> {
    require_local_desktop_app()?;

    let group_meta = group_db::add_empty_group(
        &state.app_state.db,
        &state.get_current_user(),
        group_name,
        None,
    )
    .await?;

    Ok(group_meta)
}

#[tauri::command]
pub async fn add_models_to_group(
    group_id: i64,
    model_ids: Vec<i64>,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    require_local_desktop_app()?;

    group_db::set_group_id_on_models(
        &state.app_state.db,
        &state.get_current_user(),
        Some(group_id),
        model_ids,
        None,
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn remove_models_from_group(
    model_ids: Vec<i64>,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    require_local_desktop_app()?;

    group_db::set_group_id_on_models(
        &state.app_state.db,
        &state.get_current_user(),
        None,
        model_ids,
        None,
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn edit_group(
    group_id: i64,
    group_name: &str,
    group_timestamp: Option<&str>,
    group_global_id: Option<&str>,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    require_local_desktop_app()?;

    group_db::edit_group(
        &state.app_state.db,
        &state.get_current_user(),
        group_id,
        group_name,
        group_timestamp,
        group_global_id,
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn get_group_count(
    include_ungrouped_models: Option<bool>,
    state: State<'_, TauriAppState>,
) -> Result<usize, ApplicationError> {
    require_local_desktop_app()?;

    let count = group_db::get_group_count(
        &state.app_state.db,
        &state.get_current_user(),
        include_ungrouped_models.unwrap_or(false),
    )
    .await?;

    Ok(count)
}
