//! The shared journal retains creation evidence; it is not a complete sync frontier.
use db::{
    causal_envelope::{
        self, Action, CausalEnvelope, CreationCompleteness, Dot, Entity, EntityKind,
    },
    causal_store::{self, ReceiveResult},
    group_db,
    group_history::ReplicaPrincipal,
    model::user::User,
    model_db, random_hex_32, user_db,
};

mod common;

#[tokio::test]
async fn local_creation_uses_shared_journal_and_rolls_back_on_failure() {
    let (_dir, db) = common::test_db().await;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'causal_events')").fetch_one(&db).await.unwrap();
    assert!(exists, "creation records need the shared event journal");
    let user = User::default();
    let blob = db::blob_db::add_blob(&db, "journal-model", "stl", 1, None)
        .await
        .unwrap();
    let before: i64 =
        sqlx::query_scalar("SELECT counter FROM group_causal_replica WHERE singleton = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    sqlx::query("CREATE TRIGGER reject_journal BEFORE INSERT ON causal_events BEGIN SELECT RAISE(ABORT, 'injected journal failure'); END").execute(&db).await.unwrap();
    assert!(
        group_db::add_empty_group(&db, &user, "failure", None)
            .await
            .is_err()
    );
    assert!(
        model_db::add_model(&db, &user, "failure", blob, None, None)
            .await
            .is_err()
    );
    let counts: (i64, i64, i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group), (SELECT COUNT(*) FROM models), (SELECT COUNT(*) FROM entity_births), (SELECT COUNT(*) FROM causal_events), (SELECT counter FROM group_causal_replica WHERE singleton = 1)").fetch_one(&db).await.unwrap();
    assert_eq!(counts, (0, 0, 0, 0, before));
    sqlx::query("DROP TRIGGER reject_journal")
        .execute(&db)
        .await
        .unwrap();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "part", blob, None, None)
        .await
        .unwrap();
    let group_birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let model_birth: String =
        sqlx::query_scalar("SELECT model_birth_id FROM models WHERE model_id = ?")
            .bind(model)
            .fetch_one(&db)
            .await
            .unwrap();
    let mut dots = Vec::new();
    for (kind, birth) in [
        (EntityKind::Group, group_birth),
        (EntityKind::Model, model_birth),
    ] {
        let record = causal_envelope::get_local_creation_record(&db, &user, kind, &birth)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.completeness, CreationCompleteness::Incomplete);
        let bytes: String = sqlx::query_scalar("SELECT envelope FROM causal_events WHERE user_id = ? AND writer_id = ? AND counter = ?")
            .bind(user.id).bind(&record.envelope.event.writer_id).bind(record.envelope.event.counter.parse::<i64>().unwrap()).fetch_one(&db).await.unwrap();
        assert_eq!(
            bytes,
            causal_envelope::canonical_causal_envelope(&record.envelope).unwrap()
        );
        dots.push(record.envelope.event);
    }
    assert_ne!(dots[0], dots[1]);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn migration_preserves_creation_and_receipt_bytes_dots_and_unknown_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|item| item.version < 20_261_113_110_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let birth = random_hex_32();
    let global = random_hex_32();
    let writer = random_hex_32();
    sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id, group_birth_id) VALUES ('before', '2020-01-01T00:00:00Z', ?, '2020-01-01T00:00:00Z', ?, ?)").bind(user.id).bind(&global).bind(&birth).execute(&db).await.unwrap();
    let entity = Entity {
        kind: EntityKind::Group,
        global_id: global,
        birth_id: Some(birth.clone()),
    };
    let creation = CausalEnvelope {
        version: 2,
        entity: entity.clone(),
        event: Dot {
            writer_id: writer.clone(),
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![],
        action: Action::GroupCreated {
            name: "before".into(),
        },
    };
    let creation_bytes = causal_envelope::canonical_causal_envelope(&creation).unwrap();
    sqlx::query("INSERT INTO local_creation_records (user_id,entity_kind,birth_id,writer_id,counter,envelope) VALUES (?, 'group', ?, ?, 1, ?)").bind(user.id).bind(&birth).bind(&writer).bind(&creation_bytes).execute(&db).await.unwrap();
    sqlx::query("UPDATE entity_births SET initialization_status = 'created', initialization_writer_id = ?, initialization_counter = 1 WHERE user_id = ? AND entity_kind = 'group' AND birth_id = ?").bind(&writer).bind(user.id).bind(&birth).execute(&db).await.unwrap();
    let unknown_birth = random_hex_32();
    sqlx::query("INSERT INTO models_group (group_name, group_created, group_user_id, group_last_modified, group_unique_global_id, group_birth_id) VALUES ('old', '2020-01-01T00:00:00Z', ?, '2020-01-01T00:00:00Z', ?, ?)").bind(user.id).bind(random_hex_32()).bind(&unknown_birth).execute(&db).await.unwrap();
    let remote = random_hex_32();
    sqlx::query("INSERT INTO causal_replica_owners (user_id,replica_id) VALUES (?,?)")
        .bind(user.id)
        .bind(&remote)
        .execute(&db)
        .await
        .unwrap();
    let receipt = CausalEnvelope {
        version: 2,
        entity,
        event: Dot {
            writer_id: remote.clone(),
            counter: "1".into(),
        },
        parents: vec![creation.event.clone()],
        dependencies: vec![],
        action: Action::GroupRenamed {
            name: "remote".into(),
        },
    };
    let receipt_bytes = causal_envelope::canonical_causal_envelope(&receipt).unwrap();
    sqlx::query(
        "INSERT INTO causal_event_receipts (user_id,writer_id,counter,envelope) VALUES (?,?,1,?)",
    )
    .bind(user.id)
    .bind(&remote)
    .bind(&receipt_bytes)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO causal_receipt_dependencies (user_id,writer_id,counter,dependency_writer_id,dependency_counter) VALUES (?,?,1,?,1)").bind(user.id).bind(&remote).bind(&writer).execute(&db).await.unwrap();
    db.close().await;
    let db = db::db_context::setup_db(&path, &dir.path().join("backups")).await;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'causal_events')").fetch_one(&db).await.unwrap();
    assert!(exists, "migration must create the common journal");
    for (event_writer, expected) in [(&writer, &creation_bytes), (&remote, &receipt_bytes)] {
        let bytes: String = sqlx::query_scalar("SELECT envelope FROM causal_events WHERE user_id = ? AND writer_id = ? AND counter = 1").bind(user.id).bind(event_writer).fetch_one(&db).await.unwrap();
        assert_eq!(&bytes, expected);
    }
    let record = causal_envelope::get_local_creation_record(&db, &user, EntityKind::Group, &birth)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.envelope, creation);
    assert_eq!(record.completeness, CreationCompleteness::Incomplete);
    assert!(
        causal_envelope::get_local_creation_record(&db, &user, EntityKind::Group, &unknown_birth)
            .await
            .unwrap()
            .is_none()
    );
    let init: (String, Option<String>, Option<i64>) = sqlx::query_as("SELECT initialization_status,initialization_writer_id,initialization_counter FROM entity_births WHERE user_id = ? AND entity_kind = 'group' AND birth_id = ?").bind(user.id).bind(&birth).fetch_one(&db).await.unwrap();
    assert_eq!(init, ("created".into(), Some(writer), Some(1)));
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let stored = causal_store::get_received(&db, &user, &identity, &receipt.event)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.envelope, receipt);
    assert!(
        stored.missing.is_empty(),
        "migrated creation resolves the original dependency"
    );
    let principal = ReplicaPrincipal {
        owner_user_id: user.id,
        replica_id: remote,
    };
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &principal, &receipt_bytes)
            .await
            .unwrap(),
        ReceiveResult::AlreadyPresent
    ));
}

#[tokio::test]
async fn duplicate_dot_between_old_stores_aborts_journal_migration_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|item| item.version < 20_261_113_110_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let birth = random_hex_32();
    let global = random_hex_32();
    let writer = random_hex_32();
    sqlx::query("INSERT INTO models_group (group_name,group_created,group_user_id,group_last_modified,group_unique_global_id,group_birth_id) VALUES ('old','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?,?)").bind(user.id).bind(&global).bind(&birth).execute(&db).await.unwrap();
    let event = CausalEnvelope {
        version: 2,
        entity: Entity {
            kind: EntityKind::Group,
            global_id: global,
            birth_id: Some(birth.clone()),
        },
        event: Dot {
            writer_id: writer.clone(),
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![],
        action: Action::GroupCreated { name: "old".into() },
    };
    let bytes = causal_envelope::canonical_causal_envelope(&event).unwrap();
    sqlx::query("INSERT INTO local_creation_records (user_id,entity_kind,birth_id,writer_id,counter,envelope) VALUES (?,'group',?,?,1,?)").bind(user.id).bind(&birth).bind(&writer).bind(&bytes).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO causal_replica_owners (user_id,replica_id) VALUES (?,?)")
        .bind(user.id)
        .bind(&writer)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO causal_event_receipts (user_id,writer_id,counter,envelope) VALUES (?,?,1,?)",
    )
    .bind(user.id)
    .bind(&writer)
    .bind(&bytes)
    .execute(&db)
    .await
    .unwrap();
    assert!(
        sqlx::migrate!("./migrations").run(&db).await.is_err(),
        "even identical dot collision must fail rather than merge provenance"
    );
    let common: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='causal_events')",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(!common, "failed migration must roll back its schema");
    let original: (String, String) = sqlx::query_as("SELECT (SELECT envelope FROM local_creation_records WHERE writer_id=?), (SELECT envelope FROM causal_event_receipts WHERE writer_id=?)").bind(&writer).bind(&writer).fetch_one(&db).await.unwrap();
    assert_eq!(original, (bytes.clone(), bytes));
}

#[tokio::test]
async fn account_deletion_cascades_journal_receipts_and_dependencies() {
    let (_dir, db) = common::test_db().await;
    let id = user_db::add_user(
        &db,
        "journal-owner",
        "journal-owner@example.test",
        &random_hex_32(),
    )
    .await
    .unwrap();
    let user = user_db::get_user_by_id(&db, id).await.unwrap().unwrap();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id=?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let writer = random_hex_32();
    sqlx::query("INSERT INTO causal_replica_owners (user_id,replica_id) VALUES (?,?)")
        .bind(id)
        .bind(&writer)
        .execute(&db)
        .await
        .unwrap();
    let principal = ReplicaPrincipal {
        owner_user_id: id,
        replica_id: writer.clone(),
    };
    let identity = user_db::get_sync_identity(&db, id).await.unwrap().unwrap();
    let event = CausalEnvelope {
        version: 2,
        entity: Entity {
            kind: EntityKind::Group,
            global_id: group.unique_global_id,
            birth_id: Some(birth),
        },
        event: Dot {
            writer_id: writer.clone(),
            counter: "1".into(),
        },
        parents: vec![Dot {
            writer_id: random_hex_32(),
            counter: "1".into(),
        }],
        dependencies: vec![],
        action: Action::GroupRenamed {
            name: "pending".into(),
        },
    };
    let wire = causal_envelope::canonical_causal_envelope(&event).unwrap();
    assert!(matches!(
        causal_store::receive(&db, &user, &identity, &principal, &wire)
            .await
            .unwrap(),
        ReceiveResult::AcceptedPending { .. }
    ));
    sqlx::query("DELETE FROM users WHERE user_id=?")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    let counts: (i64, i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events WHERE user_id=?), (SELECT COUNT(*) FROM local_creation_records WHERE user_id=?), (SELECT COUNT(*) FROM causal_event_receipts WHERE user_id=?), (SELECT COUNT(*) FROM causal_receipt_dependencies WHERE user_id=?)").bind(id).bind(id).bind(id).bind(id).fetch_one(&db).await.unwrap();
    assert_eq!(counts, (0, 0, 0, 0));
}
