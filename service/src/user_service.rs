/// Deletes the user, then the files of blobs nothing references any more.
pub async fn delete_user(
    app_state: &crate::AppState,
    user_id: i64,
) -> Result<(), crate::ServiceError> {
    db::user_db::delete_user(&app_state.db, user_id).await?;

    crate::export_service::delete_dead_blobs(app_state).await
}
