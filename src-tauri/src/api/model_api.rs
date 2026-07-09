use std::str::FromStr;

use serde::Serialize;
use tauri::{AppHandle, State};

use db::{
    model::ModelFlags,
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

    let models_len = models.len();
    let (_, paths) =
        export_service::export_to_temp_folder(models, &state.app_state, true, "open").await?;

    if open_in_slicer
        && models_len > 0
        && let Some(slicer) = &state.get_configuration().slicer
    {
        slicer.open(paths, &state.app_state).await?;
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
                .map_err(|_| ApplicationError::InternalError(
                    "Invalid order_by value. Valid values are: AddedAsc, AddedDesc, NameAsc, NameDesc, SizeAsc, SizeDesc, ModifiedAsc, ModifiedDesc".to_string()
                ))?,
            model_flags,
            text_search,
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
        model_flags.unwrap_or(ModelFlags::empty()),
        model_timestamp,
        model_global_id,
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn delete_model(
    model_id: i64,
    state: State<'_, TauriAppState>,
) -> Result<(), ApplicationError> {
    export_service::delete_models(&state.app_state, &state.get_current_user(), vec![model_id])
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
pub async fn get_model_count(
    flags: Option<ModelFlags>,
    state: State<'_, TauriAppState>,
) -> Result<usize, ApplicationError> {
    let count =
        model_db::get_model_count(&state.app_state.db, &state.get_current_user(), flags).await?;

    Ok(count)
}

#[derive(Debug, Serialize)]
pub struct ModelDiskSpaceUsage {
    pub size_uncompressed: u64,
    pub size_compressed: u64,
}

#[tauri::command]
pub async fn get_model_disk_space_usage(
    state: State<'_, TauriAppState>,
) -> Result<ModelDiskSpaceUsage, ApplicationError> {
    let data = model_db::get_size_of_models(&state.app_state.db, &state.get_current_user()).await?;
    let local = export_service::get_size_of_blobs(&data.blob_sha256, &state.app_state)?;

    Ok(ModelDiskSpaceUsage {
        size_uncompressed: u64::try_from(data.total_size).unwrap_or(0),
        size_compressed: local,
    })
}
