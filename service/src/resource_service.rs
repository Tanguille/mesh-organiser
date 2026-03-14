use db::model::{resource::ResourceMeta, user::User};

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
