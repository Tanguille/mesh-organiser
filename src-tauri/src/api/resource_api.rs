use tauri::State;

use db::{
    model::{
        model_group::ModelGroup,
        resource::{ResourceFlags, ResourceMeta},
    },
    resource_db,
};
use service::resource_service;

use crate::{error::ApplicationError, tauri_app_state::TauriAppState};

#[tauri::command]
pub async fn get_resources(
    state: State<'_, TauriAppState>,
) -> Result<Vec<ResourceMeta>, ApplicationError> {
    let resources =
        resource_db::get_resources(&state.app_state.db, &state.get_current_user()).await?;

    Ok(resources)
}

#[tauri::command]
pub async fn add_resource(
    resource_name: &str,
    state: State<'_, TauriAppState>,
) -> Result<ResourceMeta, ApplicationError> {
    let resource_meta = resource_db::add_resource(
        &state.app_state.db,
        &state.get_current_user(),
        resource_name,
        None,
    )
    .await?;

    Ok(resource_meta)
}

#[tauri::command]
pub async fn edit_resource(
    resource_id: i64,
    resource_name: &str,
    resource_flags: ResourceFlags,
    resource_timestamp: Option<&str>,
    resource_global_id: Option<&str>,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    resource_db::edit_resource(
        &state.app_state.db,
        &state.get_current_user(),
        resource_id,
        resource_name,
        resource_flags,
        resource_timestamp,
        resource_global_id,
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn remove_resource(
    resource_id: i64,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    resource_service::delete_resource(resource_id, &state.get_current_user(), &state.app_state)
        .await?;

    Ok(())
}

#[tauri::command]
pub async fn open_resource_folder(
    resource_id: i64,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    let user = state.get_current_user();
    let resource =
        resource_db::get_resource_meta_by_id(&state.app_state.db, &user, resource_id).await?;

    if resource.is_none() {
        return Err(ApplicationError::InternalError(String::from(
            "Resource not found",
        )));
    }

    let resource = resource.unwrap();

    resource_service::open_resource_folder(&resource, &user, &state.app_state)?;
    Ok(())
}

#[tauri::command]
pub async fn set_resource_on_group(
    resource_id: Option<i64>,
    group_id: i64,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    resource_db::set_resource_on_group(
        &state.app_state.db,
        &state.get_current_user(),
        resource_id,
        group_id,
        None,
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn get_groups_for_resource(
    resource_id: i64,
    state: State<'_, TauriAppState>,
) -> Result<Vec<ModelGroup>, ApplicationError> {
    let groups = resource_db::get_groups_for_resource(
        &state.app_state.db,
        &state.get_current_user(),
        resource_id,
    )
    .await?;

    Ok(groups)
}
