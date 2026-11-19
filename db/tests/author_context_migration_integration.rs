//! Historical operations retain their original bytes and cannot invent an author cut.
#[tokio::test]
async fn legacy_operations_keep_exact_event_order_and_unknown_context() {
    let dir = tempfile::tempdir().unwrap();
    let db = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(dir.path().join("db.sqlite"))
            .create_if_missing(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|migration| migration.version < 20_261_113_260_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let writer = db::random_hex_32();
    let operation = db::random_hex_32();
    let birth = db::random_hex_32();
    let wires = [r#"{"old":"first"}"#, r#"{ "old" : "second" }"#];
    sqlx::query("INSERT INTO group_lifecycle_operations(user_id,operation_id,focus_birth_id,event_count) VALUES(1,?,?,2)").bind(&operation).bind(&birth).execute(&db).await.unwrap();
    for (index, wire) in wires.iter().enumerate() {
        let counter = i64::try_from(index + 1).unwrap();
        sqlx::query(
            "INSERT INTO causal_events(user_id,writer_id,counter,envelope) VALUES(1,?,?,?)",
        )
        .bind(&writer)
        .bind(counter)
        .bind(wire)
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO group_lifecycle_events(user_id,operation_id,position,writer_id,counter) VALUES(1,?,?,?,?)").bind(&operation).bind(i64::try_from(index).unwrap()).bind(&writer).bind(counter).execute(&db).await.unwrap();
    }
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    let row: (String, i64, Option<String>) = sqlx::query_as("SELECT focus_birth_id,event_count,author_context FROM group_lifecycle_operations WHERE user_id=1 AND operation_id=?").bind(&operation).fetch_one(&db).await.unwrap();
    assert_eq!(row, (birth, 2, None));
    let actual: Vec<String> = sqlx::query_scalar("SELECT c.envelope FROM group_lifecycle_events e JOIN causal_events c USING(user_id,writer_id,counter) WHERE e.user_id=1 AND e.operation_id=? ORDER BY e.position").bind(&operation).fetch_all(&db).await.unwrap();
    assert_eq!(actual, wires);
    assert!(sqlx::query("UPDATE group_lifecycle_operations SET author_context='{}' WHERE user_id=1 AND operation_id=?").bind(&operation).execute(&db).await.is_err(), "legacy context cannot be backfilled");
}
