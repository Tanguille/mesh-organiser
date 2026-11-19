//! Upgrade must retain the original transport receipt and canonical event bytes.
use db::random_hex_32;

#[tokio::test]
async fn transport_log_upgrade_preserves_original_author_sequence_digest_and_event() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|migration| migration.version < 20_261_113_250_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let writer = random_hex_32();
    let account: String =
        sqlx::query_scalar("SELECT account_id FROM sync_accounts WHERE user_id=1")
            .fetch_one(&db)
            .await
            .unwrap();
    sqlx::query("INSERT INTO sync_transport_writers(writer_id,user_id,account_id,connection_id,secret_hash,client_instance_id,client_account_id) VALUES (?,1,?,?,?,?,?)").bind(&writer).bind(&account).bind(random_hex_32()).bind("a".repeat(64)).bind(random_hex_32()).bind(random_hex_32()).execute(&db).await.unwrap();
    let birth = random_hex_32();
    let envelope = db::causal_envelope::CausalEnvelope {
        version: 2,
        entity: db::causal_envelope::Entity {
            kind: db::causal_envelope::EntityKind::Group,
            global_id: random_hex_32(),
            birth_id: Some(birth.clone()),
        },
        event: db::causal_envelope::Dot {
            writer_id: writer.clone(),
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![],
        action: db::causal_envelope::Action::GroupCreated {
            name: "original".into(),
        },
    };
    let bytes = db::causal_envelope::canonical_causal_envelope(&envelope).unwrap();
    sqlx::query("INSERT INTO causal_events(user_id,writer_id,counter,envelope) VALUES (1,?,1,?)")
        .bind(&writer)
        .bind(&bytes)
        .execute(&db)
        .await
        .unwrap();
    let digest = "b".repeat(64);
    sqlx::query("INSERT INTO sync_transport_log(user_id,sequence,kind,item_id,digest,author_writer_id) VALUES (1,7,'group_genesis',?,?,?)").bind(&birth).bind(&digest).bind(&writer).execute(&db).await.unwrap();
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    let row: (i64,String,String,String,String) = sqlx::query_as("SELECT sequence,kind,item_id,digest,authority_writer_id FROM sync_transport_log WHERE user_id=1").fetch_one(&db).await.unwrap();
    assert_eq!(
        row,
        (7, "group_genesis".into(), birth, digest, writer.clone())
    );
    let original: String =
        sqlx::query_scalar("SELECT writer_id FROM sync_transport_log WHERE user_id=1")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(original, writer);
    let after: String = sqlx::query_scalar(
        "SELECT envelope FROM causal_events WHERE user_id=1 AND writer_id=? AND counter=1",
    )
    .bind(&original)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(after, bytes);
}
