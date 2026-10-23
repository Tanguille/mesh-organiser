use std::str::FromStr;

use tauri::{AppHandle, State};

use db::{
    model::{ModelFlags, blob::FileType},
    model_db,
    model_db::{ModelFilterOptions, ModelOrderBy},
};
use service::{export_service, import_service, import_state::ImportStatus, thumbnail_service};

use crate::{
    ImportState, TauriAppState, error::ApplicationError, tauri_import_state::import_state_new_tauri,
};

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
#[tauri::command]
pub async fn add_model(
    path: &str,
    recursive: bool,
    delete_imported: bool,
    import_as_path: bool,
    origin_url: Option<String>,
    open_in_slicer: bool,
    state: State<'_, TauriAppState>,
    app_handle: AppHandle,
) -> Result<ImportState, ApplicationError> {
    let mut import_state = import_state_new_tauri(
        origin_url,
        recursive,
        delete_imported,
        import_as_path,
        &state,
        &app_handle,
    );
    import_state = import_service::import_path(path, &state.app_state, import_state).await?;

    let model_ids = import_state.all_model_ids();

    let models = thumbnail_service::generate_thumbnails_for_model_ids(
        &state.app_state,
        &state.get_current_user(),
        model_ids,
        &mut import_state,
    )
    .await?;

    if open_in_slicer
        && !models.is_empty()
        && let Some(slicer) = &state.get_configuration().slicer
    {
        let (_, paths) =
            export_service::export_to_temp_folder(models, &state.app_state, true, "open").await?;
        slicer.open(paths, &state.app_state)?;
    }

    import_state.status = ImportStatus::Finished;
    Ok(import_state)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn get_models(
    model_ids: Option<Vec<i64>>,
    group_ids: Option<Vec<i64>>,
    label_ids: Option<Vec<i64>>,
    order_by: Option<String>,
    text_search: Option<String>,
    model_flags: Option<ModelFlags>,
    file_types: Option<Vec<FileType>>,
    page: u32,
    page_size: u32,
    state: State<'_, TauriAppState>,
) -> Result<Vec<db::model::Model>, ApplicationError> {
    let models = model_db::get_models(
        &state.app_state.db,
        &state.get_current_user(),
        ModelFilterOptions {
            model_ids,
            group_ids,
            label_ids,
            order_by: order_by
                .map(|s| ModelOrderBy::from_str(&s))
                .transpose()
                .map_err(|_| service::ServiceError::InternalError(
                    "Invalid order_by value. Valid values are: AddedAsc, AddedDesc, NameAsc, NameDesc, SizeAsc, SizeDesc, ModifiedAsc, ModifiedDesc".to_string()
                ))?,
            model_flags,
            text_search,
            file_types,
            trashed: false,
            page,
            page_size,
        },
    )
    .await?;

    Ok(models.items)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn edit_model(
    model_id: i64,
    model_name: &str,
    model_url: Option<&str>,
    model_description: Option<&str>,
    model_flags: Option<ModelFlags>,
    model_timestamp: Option<&str>,
    model_global_id: Option<&str>,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    model_db::edit_model(
        &state.app_state.db,
        &state.get_current_user(),
        model_id,
        model_name,
        model_url,
        model_description,
        model_flags.unwrap_or_default(),
        model_timestamp,
        model_global_id,
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn delete_models(
    model_ids: Vec<i64>,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    export_service::delete_models(&state.app_state, &state.get_current_user(), model_ids).await?;
    Ok(())
}

#[tauri::command]
pub async fn get_trashed_models(
    state: State<'_, TauriAppState>,
) -> Result<Vec<db::model::Model>, ApplicationError> {
    Ok(export_service::get_trashed_models(&state.app_state, &state.get_current_user()).await?)
}

#[tauri::command]
pub async fn restore_models(
    model_ids: Vec<i64>,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    export_service::restore_models(&state.app_state, &state.get_current_user(), model_ids).await?;
    Ok(())
}

#[tauri::command]
pub async fn get_model_count(
    flags: Option<ModelFlags>,
    state: State<'_, TauriAppState>,
) -> Result<usize, ApplicationError> {
    Ok(model_db::get_model_count(&state.app_state.db, &state.get_current_user(), flags).await?)
}

#[tauri::command]
pub async fn get_model_disk_space_usage(
    state: State<'_, TauriAppState>,
) -> Result<export_service::ModelDiskSpaceUsage, ApplicationError> {
    Ok(
        export_service::get_model_disk_space_usage(&state.app_state, &state.get_current_user())
            .await?,
    )
}
