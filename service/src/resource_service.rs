use db::{
    model::{resource::ResourceMeta, user::User},
    resource_db,
};

use crate::{service_error::ServiceError, util::open_folder_in_explorer};

use super::app_state::AppState;

/// Opens the resource folder in the system file explorer.
///
/// # Errors
///
/// Returns an error if the path cannot be created or opened.
pub fn open_resource_folder(
    resource: &ResourceMeta,
    user: &User,
    app_state: &AppState,
) -> Result<(), ServiceError> {
    let path = app_state.get_resources_dir();
    let resource_path = path.join(format!("{}_{}", resource.id, user.id));

    if !resource_path.exists() {
        let old_resource_path = path.join(resource.id.to_string());
        if old_resource_path.exists() {
            std::fs::rename(&old_resource_path, &resource_path)?;
        } else {
            std::fs::create_dir_all(&resource_path)?;
        }
    }

    open_folder_in_explorer(&resource_path);

    Ok(())
}

/// Deletes the resource folder for the user.
///
/// # Errors
///
/// Returns an error if the directory cannot be removed.
pub fn delete_resource_folder(
    resource: &ResourceMeta,
    user: &User,
    app_state: &AppState,
) -> Result<(), ServiceError> {
    let mut path = app_state.get_resources_dir();
    path.push(format!("{}_{}", resource.id, user.id));

    if path.exists() {
        std::fs::remove_dir_all(&path)?;
    }

    Ok(())
}

/// Deletes a resource: removes its folder on disk, then the database row.
/// Shared by the Tauri command and the web handler.
///
/// # Errors
///
/// Returns an error if the resource does not exist for the user, the folder
/// cannot be removed, or the database delete fails.
pub async fn delete_resource(
    resource_id: i64,
    user: &User,
    app_state: &AppState,
) -> Result<(), ServiceError> {
    let Some(resource) =
        resource_db::get_resource_meta_by_id(&app_state.db, user, resource_id).await?
    else {
        return Err(ServiceError::InternalError(String::from(
            "Resource not found",
        )));
    };

    delete_resource_folder(&resource, user, app_state)?;
    resource_db::delete_resource(&app_state.db, user, resource.id).await?;

    Ok(())
}
