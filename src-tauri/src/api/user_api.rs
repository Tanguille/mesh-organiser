use tauri::State;

use db::{model::user::User, user_db};
use service::user_service;

use crate::{
    error::ApplicationError, mobile_guard::require_local_desktop_app,
    tauri_app_state::TauriAppState,
};

#[tauri::command]
pub async fn get_current_user(state: State<'_, TauriAppState>) -> Result<User, ApplicationError> {
    require_local_desktop_app()?;

    let user = state.get_current_user();

    Ok(user)
}

#[tauri::command]
pub async fn set_current_user(
    user_id: i64,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    require_local_desktop_app()?;

    state.set_current_user_by_id(user_id).await?;

    Ok(())
}

#[tauri::command]
pub async fn get_users(state: State<'_, TauriAppState>) -> Result<Vec<User>, ApplicationError> {
    require_local_desktop_app()?;

    Ok(user_db::get_users(&state.app_state.db).await?)
}

#[tauri::command]
pub async fn add_user(
    user_name: &str,
    user_email: &str,
    user_password: &str,
    state: State<'_, TauriAppState>,
) -> Result<i64, ApplicationError> {
    require_local_desktop_app()?;

    Ok(user_db::add_user(&state.app_state.db, user_name, user_email, user_password).await?)
}

#[tauri::command]
pub async fn edit_user(
    user_id: i64,
    user_name: &str,
    user_email: &str,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    require_local_desktop_app()?;

    user_db::edit_user_min(&state.app_state.db, user_id, user_name, user_email).await?;

    Ok(())
}

#[tauri::command]
pub async fn set_last_sync_time(
    user_id: i64,
    user_last_sync: &str,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    require_local_desktop_app()?;

    user_db::edit_user_last_sync_time(&state.app_state.db, user_id, user_last_sync).await?;

    Ok(())
}

#[tauri::command]
pub async fn set_sync_state(
    user_sync_token: String,
    user_sync_url: String,
    online: bool,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    require_local_desktop_app()?;

    user_db::set_user_sync_token(
        &state.app_state.db,
        state.get_current_user().id,
        &user_sync_token,
        &user_sync_url,
        online,
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn unset_sync_state(state: State<'_, TauriAppState>) -> Result<(), ApplicationError> {
    require_local_desktop_app()?;

    user_db::clear_user_sync(&state.app_state.db, state.get_current_user().id).await?;

    Ok(())
}

#[tauri::command]
pub async fn delete_user(
    user_id: i64,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    require_local_desktop_app()?;

    if state.get_current_user().id == user_id {
        return Err(service::ServiceError::InternalError(
            "Cannot delete the currently logged in user.".into(),
        )
        .into());
    }

    user_service::delete_user(&state.app_state, user_id).await?;

    Ok(())
}
