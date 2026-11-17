#[tokio::test]
async fn memory_database_cannot_certify_a_filesystem_owner() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();

    assert!(
        db::sync_transport::stage_slots::database_path(&pool)
            .await
            .is_err()
    );
}
