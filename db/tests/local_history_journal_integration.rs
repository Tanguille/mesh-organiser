//! Local-only evidence is never a portable or complete causal history.
mod common;
use db::{
    blob_db, group_db,
    group_history::{self, HistoryOrigin},
    model::user::User,
    model_db,
};

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn real_history_chain_is_stored_as_local_only_and_keeps_existing_readers() {
    let (_dir, db) = common::test_db().await;
    let has_encoding: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('causal_events') WHERE name='encoding')",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(
        has_encoding,
        "journal must distinguish local-only histories from portable events"
    );
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, "local-journal", "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "model", blob, None, None)
        .await
        .unwrap();
    let (global, incarnation): (String, String) = sqlx::query_as(
        "SELECT model_unique_global_id,model_incarnation FROM models WHERE model_id=?",
    )
    .bind(model)
    .fetch_one(&db)
    .await
    .unwrap();
    group_db::edit_group(&db, &user, group.id, "renamed", None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let request = common::group_restore_request(&db, &user, group.id).await;
    group_db::restore_groups(&db, &user, std::slice::from_ref(&request), 30)
        .await
        .unwrap();
    let group_history = group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        group_history.ancestry,
        HistoryOrigin::UnknownPreUpgrade
    ));
    let lifecycle = group_history::get_group_lifecycle_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    let additions =
        group_history::get_group_member_added_history(&db, &user, &group.unique_global_id)
            .await
            .unwrap()
            .unwrap();
    let membership = group_history::get_membership_history(&db, &user, &global, &incarnation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group_history.events.len(), 1);
    assert_eq!(lifecycle.len(), 2);
    assert_eq!(additions.len(), 1);
    assert_eq!(membership.events.len(), 3);
    assert_eq!(additions[0].membership_event, membership.events[0].id);
    assert_eq!(lifecycle[0].parents, vec![additions[0].id.clone()]);
    assert_eq!(lifecycle[1].parents, vec![lifecycle[0].id.clone()]);
    assert_eq!(
        membership.events[1].parents,
        vec![membership.events[0].id.clone()]
    );
    assert_eq!(
        membership.events[2].parents,
        vec![membership.events[1].id.clone()]
    );
    assert_eq!(group_history.frontier, vec![lifecycle[1].id.clone()]);
    assert_eq!(membership.frontier, vec![membership.events[2].id.clone()]);
    let mut dots = vec![
        group_history.events[0].id.clone(),
        additions[0].id.clone(),
        lifecycle[0].id.clone(),
        lifecycle[1].id.clone(),
    ];
    dots.extend(membership.events.iter().map(|event| event.id.clone()));
    for dot in dots {
        let encoding: String = sqlx::query_scalar(
            "SELECT encoding FROM causal_events WHERE user_id=? AND writer_id=? AND counter=?",
        )
        .bind(user.id)
        .bind(&dot.replica_id)
        .bind(dot.counter)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(encoding, "local_history_v1");
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM causal_events WHERE user_id=? AND encoding='local_history_v1'",
    )
    .bind(user.id)
    .fetch_one(&db)
    .await
    .unwrap();
    group_db::restore_groups(&db, &user, &[request], 30)
        .await
        .unwrap();
    let after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM causal_events WHERE user_id=? AND encoding='local_history_v1'",
    )
    .bind(user.id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(after, count, "retry must not invent events");
}

#[tokio::test]
async fn rejected_local_journal_write_rolls_back_real_rename_and_counter() {
    let (_dir, db) = common::test_db().await;
    let has_encoding: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('causal_events') WHERE name='encoding')",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(has_encoding, "local history must use the shared journal");
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "before", None)
        .await
        .unwrap();
    let counter: i64 =
        sqlx::query_scalar("SELECT counter FROM group_causal_replica WHERE singleton=1")
            .fetch_one(&db)
            .await
            .unwrap();
    sqlx::query("CREATE TRIGGER reject_local_history BEFORE INSERT ON causal_events WHEN NEW.encoding='local_history_v1' BEGIN SELECT RAISE(ABORT, 'injected failure'); END").execute(&db).await.unwrap();
    assert!(
        group_db::edit_group(&db, &user, group.id, "after", None, None)
            .await
            .is_err()
    );
    let (name, actual): (String, i64) = sqlx::query_as("SELECT group_name,(SELECT counter FROM group_causal_replica WHERE singleton=1) FROM models_group WHERE group_id=?").bind(group.id).fetch_one(&db).await.unwrap();
    assert_eq!((name, actual), ("before".into(), counter));
    let history = group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(history.events, []);
    assert_eq!(history.frontier, []);
}

#[tokio::test]
async fn portable_receipt_cannot_depend_on_local_only_history() {
    use db::{
        causal_envelope::{self, Action, CausalEnvelope, Dot, Entity, EntityKind},
        causal_store,
        group_history::ReplicaPrincipal,
        random_hex_32, user_db,
    };
    let (_dir, db) = common::test_db().await;
    let has_encoding: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('causal_events') WHERE name='encoding')",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(
        has_encoding,
        "local and portable events must have distinct encodings"
    );
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "before", None)
        .await
        .unwrap();
    let birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id=?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    group_db::edit_group(&db, &user, group.id, "after", None, None)
        .await
        .unwrap();
    let local = group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap()
        .events[0]
        .id
        .clone();
    let writer = random_hex_32();
    sqlx::query("INSERT INTO causal_replica_owners (user_id,replica_id) VALUES (?,?)")
        .bind(user.id)
        .bind(&writer)
        .execute(&db)
        .await
        .unwrap();
    let principal = ReplicaPrincipal {
        owner_user_id: user.id,
        replica_id: writer.clone(),
    };
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let event = CausalEnvelope {
        version: 2,
        entity: Entity {
            kind: EntityKind::Group,
            global_id: group.unique_global_id,
            birth_id: Some(birth),
        },
        event: Dot {
            writer_id: writer,
            counter: "1".into(),
        },
        parents: vec![Dot {
            writer_id: local.replica_id,
            counter: local.counter.to_string(),
        }],
        dependencies: vec![],
        action: Action::GroupRenamed {
            name: "remote".into(),
        },
    };
    assert!(
        causal_store::receive(
            &db,
            &user,
            &identity,
            &principal,
            &causal_envelope::canonical_causal_envelope(&event).unwrap()
        )
        .await
        .is_err(),
        "local incomplete history cannot satisfy a portable dependency"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn migration_preserves_legacy_local_dots_edges_nulls_and_frontiers() {
    use db::random_hex_32;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version < 20_261_113_120_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let group = common::legacy_group(&db, &user, "old").await;
    let writer = random_hex_32();
    let rename = sqlx::query("INSERT INTO group_causal_events (user_id,group_global_id,replica_id,counter,name) VALUES (?,?,?,1,'renamed')").bind(user.id).bind(&group.unique_global_id).bind(&writer).execute(&db).await.unwrap().last_insert_rowid();
    let restore = sqlx::query("INSERT INTO group_causal_events (user_id,group_global_id,replica_id,counter,name,event_kind,deletion_operation_id,deletion_replica_id,deletion_counter) VALUES (?,?,?,2,'','restore','legacy-delete',NULL,NULL)").bind(user.id).bind(&group.unique_global_id).bind(&writer).execute(&db).await.unwrap().last_insert_rowid();
    sqlx::query("INSERT INTO group_causal_parents(event_id,parent_id) VALUES (?,?)")
        .bind(restore)
        .bind(rename)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO group_causal_frontier(user_id,group_global_id,event_id) VALUES (?,?,?)",
    )
    .bind(user.id)
    .bind(&group.unique_global_id)
    .bind(restore)
    .execute(&db)
    .await
    .unwrap();
    let model = common::legacy_model(&db, &user, "legacy member").await;
    let (model_global, incarnation): (String, String) = sqlx::query_as(
        "SELECT model_unique_global_id,model_incarnation FROM models WHERE model_id=?",
    )
    .bind(model)
    .fetch_one(&db)
    .await
    .unwrap();
    let first = sqlx::query("INSERT INTO membership_causal_events (user_id,model_global_id,model_incarnation,replica_id,counter,event_kind,group_global_id,deletion_operation_id,to_group_global_id) VALUES (?,?,?,?,3,'move',?,'',?)")
        .bind(user.id).bind(&model_global).bind(&incarnation).bind(&writer).bind(&group.unique_global_id).bind(&group.unique_global_id).execute(&db).await.unwrap().last_insert_rowid();
    let second = sqlx::query("INSERT INTO membership_causal_events (user_id,model_global_id,model_incarnation,replica_id,counter,event_kind,group_global_id,deletion_operation_id,from_group_global_id) VALUES (?,?,?,?,4,'move',?,'',?)")
        .bind(user.id).bind(&model_global).bind(&incarnation).bind(&writer).bind(&group.unique_global_id).bind(&group.unique_global_id).execute(&db).await.unwrap().last_insert_rowid();
    sqlx::query("INSERT INTO membership_causal_parents(event_id,parent_id) VALUES (?,?)")
        .bind(second)
        .bind(first)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO membership_causal_frontier(user_id,model_global_id,model_incarnation,event_id) VALUES (?,?,?,?)").bind(user.id).bind(&model_global).bind(&incarnation).bind(second).execute(&db).await.unwrap();
    db.close().await;
    let db = db::db_context::setup_db(&path, &dir.path().join("backups")).await;
    let has_encoding: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('causal_events') WHERE name='encoding')",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(
        has_encoding,
        "migration must place legacy events into the shared journal"
    );
    let history = group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    let lifecycle = group_history::get_group_lifecycle_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(history.ancestry, HistoryOrigin::UnknownPreUpgrade));
    assert_eq!(history.events.len(), 1);
    assert_eq!(history.events[0].name, "renamed");
    assert_eq!(
        (
            &history.events[0].id.replica_id,
            history.events[0].id.counter
        ),
        (&writer, 1)
    );
    assert_eq!(lifecycle[0].parents, vec![history.events[0].id.clone()]);
    assert_eq!(history.frontier, vec![lifecycle[0].id.clone()]);
    assert!(matches!(
        &lifecycle[0].action,
        group_history::GroupLifecycleAction::Restore {
            deletion_event: None,
            ..
        }
    ));
    let membership = group_history::get_membership_history(&db, &user, &model_global, &incarnation)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        membership.ancestry,
        HistoryOrigin::UnknownPreUpgrade
    ));
    assert_eq!(membership.events.len(), 2);
    assert_eq!(membership.events[0].id.counter, 3);
    assert_eq!(membership.events[1].id.counter, 4);
    assert_eq!(
        membership.events[1].parents,
        vec![membership.events[0].id.clone()]
    );
    assert_eq!(membership.frontier, vec![membership.events[1].id.clone()]);
    let rows: Vec<(i64,String,Option<String>,Option<String>)> = sqlx::query_as("SELECT counter,encoding,json_extract(envelope,'$.entity.birth_id'),json_extract(envelope,'$.ancestry') FROM causal_events WHERE user_id=? AND writer_id=? AND counter IN (1,2) ORDER BY counter").bind(user.id).bind(&writer).fetch_all(&db).await.unwrap();
    assert_eq!(
        rows,
        vec![
            (1, "local_history_v1".into(), None, Some("unknown".into())),
            (2, "local_history_v1".into(), None, Some("unknown".into()))
        ]
    );
}

#[tokio::test]
async fn duplicate_legacy_group_and_membership_dots_roll_back_migration() {
    use db::random_hex_32;
    let dir = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(dir.path().join("db.sqlite"))
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version < 20_261_113_120_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let group = common::legacy_group(&db, &user, "old").await;
    let model = common::legacy_model(&db, &user, "member").await;
    let (global, incarnation): (String, String) = sqlx::query_as(
        "SELECT model_unique_global_id,model_incarnation FROM models WHERE model_id=?",
    )
    .bind(model)
    .fetch_one(&db)
    .await
    .unwrap();
    let writer = random_hex_32();
    sqlx::query("INSERT INTO group_causal_events (user_id,group_global_id,replica_id,counter,name) VALUES (?,?,?,1,'name')").bind(user.id).bind(&group.unique_global_id).bind(&writer).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO membership_causal_events (user_id,model_global_id,model_incarnation,replica_id,counter,event_kind,group_global_id,deletion_operation_id) VALUES (?,?,?,?,1,'move','','')").bind(user.id).bind(&global).bind(&incarnation).bind(&writer).execute(&db).await.unwrap();
    assert!(
        sqlx::migrate!("./migrations").run(&db).await.is_err(),
        "colliding legacy dots must not be merged"
    );
    let encoding: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('causal_events') WHERE name='encoding')",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(!encoding, "failed migration must roll back its schema");
    let counts: (i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM group_causal_events WHERE replica_id=?), (SELECT COUNT(*) FROM membership_causal_events WHERE replica_id=?)").bind(&writer).bind(&writer).fetch_one(&db).await.unwrap();
    assert_eq!(counts, (1, 1));
}

#[tokio::test]
async fn account_deletion_cascades_local_history_and_journal_events() {
    use db::{random_hex_32, user_db};
    let (_dir, db) = common::test_db().await;
    let id = user_db::add_user(
        &db,
        "history-owner",
        "history-owner@example.test",
        &random_hex_32(),
    )
    .await
    .unwrap();
    let user = user_db::get_user_by_id(&db, id).await.unwrap().unwrap();
    let group = group_db::add_empty_group(&db, &user, "before", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&db, "cascade-history", "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "member", blob, None, None)
        .await
        .unwrap();
    group_db::edit_group(&db, &user, group.id, "after", None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    let local_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM causal_events WHERE user_id=? AND encoding='local_history_v1'",
    )
    .bind(id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(local_count >= 3);
    sqlx::query("DELETE FROM users WHERE user_id=?")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    let counts: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events WHERE user_id=?), (SELECT COUNT(*) FROM group_causal_events WHERE user_id=?), (SELECT COUNT(*) FROM membership_causal_events WHERE user_id=?)").bind(id).bind(id).bind(id).fetch_one(&db).await.unwrap();
    assert_eq!(counts, (0, 0, 0));
}

#[tokio::test]
async fn portable_shaped_payload_cannot_bypass_local_only_discriminator() {
    use db::{
        causal_envelope::{self, Action, CausalEnvelope, Dot, Entity, EntityKind},
        causal_store,
        group_history::ReplicaPrincipal,
        random_hex_32, user_db,
    };
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    let birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id=?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    let entity = Entity {
        kind: EntityKind::Group,
        global_id: group.unique_global_id,
        birth_id: Some(birth),
    };
    let parent = CausalEnvelope {
        version: 2,
        entity: entity.clone(),
        event: Dot {
            writer_id: random_hex_32(),
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![],
        action: Action::GroupCreated {
            name: "folder".into(),
        },
    };
    sqlx::query("INSERT INTO causal_events (user_id,writer_id,counter,encoding,envelope) VALUES (?,?,1,'local_history_v1',?)").bind(user.id).bind(&parent.event.writer_id).bind(causal_envelope::canonical_causal_envelope(&parent).unwrap()).execute(&db).await.unwrap();
    let writer = random_hex_32();
    sqlx::query("INSERT INTO causal_replica_owners (user_id,replica_id) VALUES (?,?)")
        .bind(user.id)
        .bind(&writer)
        .execute(&db)
        .await
        .unwrap();
    let principal = ReplicaPrincipal {
        owner_user_id: user.id,
        replica_id: writer.clone(),
    };
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let child = CausalEnvelope {
        version: 2,
        entity,
        event: Dot {
            writer_id: writer,
            counter: "1".into(),
        },
        parents: vec![parent.event],
        dependencies: vec![],
        action: Action::GroupRenamed {
            name: "child".into(),
        },
    };
    assert!(
        causal_store::receive(
            &db,
            &user,
            &identity,
            &principal,
            &causal_envelope::canonical_causal_envelope(&child).unwrap()
        )
        .await
        .is_err(),
        "the encoding, not payload shape, determines whether a dependency is portable"
    );
}
