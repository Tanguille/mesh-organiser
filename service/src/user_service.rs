/// Deletes the user, then the files of blobs nothing references any more.
pub async fn delete_user(
    app_state: &crate::AppState,
    user_id: i64,
) -> Result<(), crate::ServiceError> {
    db::user_db::delete_user(&app_state.db, user_id).await?;

    // Thumbnail regeneration can hold this lock for hours; startup or a later purge can
    // sweep the dead blobs instead of keeping the user deletion waiting.
    let Ok(_import_guard) = app_state.import_mutex.try_lock() else {
        return Ok(());
    };

    crate::export_service::delete_dead_blobs_locked(app_state).await
}
