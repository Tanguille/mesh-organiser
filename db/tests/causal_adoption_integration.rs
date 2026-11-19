//! In-process observations are scoped evidence, not authenticated synchronization.
use db::{
    causal_adoption,
    causal_envelope::Action,
    causal_group_lifecycle::{self, OperationImportResult},
    causal_group_rename::NameState,
    group_db,
    model::user::User,
    sync_peer, user_db,
};

mod common;

async fn assert_birth_permit_consumed(db: &db::db_context::DbContext, group_id: i64) {
    let permits: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM group_birth_adoption_permits WHERE group_id=?")
            .bind(group_id)
            .fetch_one(db)
            .await
            .unwrap();
    assert_eq!(permits, 0, "birth adoption permit must be consumed");
}

#[tokio::test]
async fn edited_legacy_group_installs_original_observation_and_can_record_later_rename() {
    let (_ad, a) = common::test_db().await;
    let (_bd, b) = common::test_db().await;
    let user = User::default();
    let ai = user_db::get_sync_identity(&a, user.id)
        .await
        .unwrap()
        .unwrap();
    let bi = user_db::get_sync_identity(&b, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = common::legacy_group(&a, &user, "before").await;
    group_db::edit_group(&a, &user, group.id, "edited", None, None)
        .await
        .unwrap();
    group_db::add_empty_group(&b, &user, "unrelated", None)
        .await
        .unwrap();
    let observation = causal_adoption::observe_group(&a, &user, &ai, group.id).await;
    assert!(
        observation.is_ok(),
        "active edited empty legacy group must be observable: {observation:?}"
    );
    let observation = observation.unwrap();
    let bundle = causal_adoption::enroll_observed_group(&a, &user, &ai, &observation)
        .await
        .unwrap();
    assert_birth_permit_consumed(&a, group.id).await;
    assert!(matches!(
        bundle.envelope().action,
        Action::LegacyObserved { .. }
    ));
    assert_eq!(bundle.source_identity(), &ai);
    assert_eq!(bundle.entity().global_id, group.unique_global_id);
    let root = bundle.event().clone();
    let repeat = causal_adoption::enroll_observed_group(&a, &user, &ai, &observation)
        .await
        .unwrap();
    assert_eq!(repeat.event(), &root);
    assert_eq!(repeat.entity(), bundle.entity());
    let peer = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let installed = causal_adoption::install_observed_group(&b, &user, &bi, &peer, &bundle)
        .await
        .unwrap();
    assert!(!installed.already_present);
    assert_ne!(installed.local_id, group.id);
    let retry = causal_adoption::install_observed_group(&b, &user, &bi, &peer, &bundle)
        .await
        .unwrap();
    assert!(retry.already_present);
    assert_eq!(retry.local_id, installed.local_id);
    let (global,birth,name):(String,Option<String>,String)=sqlx::query_as("SELECT group_unique_global_id,group_birth_id,group_name FROM models_group WHERE group_id=?").bind(installed.local_id).fetch_one(&b).await.unwrap();
    assert_eq!(global, bundle.entity().global_id);
    assert_eq!(birth, bundle.entity().birth_id);
    assert_eq!(name, "edited");
    for db in [&a, &b] {
        let wire: String = sqlx::query_scalar(
            "SELECT envelope FROM causal_events WHERE user_id=? AND writer_id=? AND counter=?",
        )
        .bind(user.id)
        .bind(&root.writer_id)
        .bind(root.counter.parse::<i64>().unwrap())
        .fetch_one(db)
        .await
        .unwrap();
        let event = db::causal_envelope::parse_causal_envelope(&wire).unwrap();
        assert!(matches!(event.action, Action::LegacyObserved { .. }));
        assert!(
            event.parents.is_empty(),
            "observation must not invent pre-observation ancestors"
        );
    }
    group_db::edit_group(&a, &user, group.id, "later", None, None)
        .await
        .unwrap();
    let state = causal_group_lifecycle::get_state(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let dot = match state.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready name: {x:?}"),
    };
    let operation = causal_group_lifecycle::export_operation(&a, &user, &ai, &dot)
        .await
        .unwrap();
    let imported = causal_group_lifecycle::import_operation(&b, &user, &bi, &peer, &operation)
        .await
        .unwrap();
    assert!(matches!(imported, OperationImportResult::Applied(_)));
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=?")
        .bind(installed.local_id)
        .fetch_one(&b)
        .await
        .unwrap();
    assert_eq!(name, "later");
}

#[tokio::test]
async fn observation_rejects_real_edits_same_second_aba_and_delete() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = common::legacy_group(&db, &user, "initial").await;
    let observation = causal_adoption::observe_group(&db, &user, &identity, group.id)
        .await
        .unwrap();
    let timestamp: String =
        sqlx::query_scalar("SELECT group_last_modified FROM models_group WHERE group_id=?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    group_db::edit_group(&db, &user, group.id, "changed", Some(&timestamp), None)
        .await
        .unwrap();
    group_db::edit_group(&db, &user, group.id, "initial", Some(&timestamp), None)
        .await
        .unwrap();
    let before:(i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
    assert!(
        causal_adoption::enroll_observed_group(&db, &user, &identity, &observation)
            .await
            .is_err(),
        "same-second ABA must invalidate the captured observation"
    );
    let after:(i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
    assert_eq!(after, before);
    let deleted = causal_adoption::observe_group(&db, &user, &identity, group.id)
        .await
        .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    assert!(
        causal_adoption::enroll_observed_group(&db, &user, &identity, &deleted)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn known_birth_observation_rejects_same_second_rename_aba() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &user, "initial", None)
        .await
        .unwrap();
    let observation = causal_adoption::observe_group(&db, &user, &identity, group.id)
        .await
        .unwrap();
    let timestamp: String =
        sqlx::query_scalar("SELECT group_last_modified FROM models_group WHERE group_id=?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    group_db::edit_group(&db, &user, group.id, "changed", Some(&timestamp), None)
        .await
        .unwrap();
    group_db::edit_group(&db, &user, group.id, "initial", Some(&timestamp), None)
        .await
        .unwrap();
    assert!(
        causal_adoption::enroll_observed_group(&db, &user, &identity, &observation)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn observation_is_owner_scoped_and_rejects_ambiguous_or_nonempty_group() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let other_id = user_db::add_user(
        &db,
        "other",
        "observation-other@example.test",
        &db::random_hex_32(),
    )
    .await
    .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = user_db::get_sync_identity(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let group = common::legacy_group(&db, &user, "source").await;
    assert!(
        causal_adoption::observe_group(&db, &other, &other_identity, group.id)
            .await
            .is_err()
    );
    assert!(
        causal_adoption::observe_group(&db, &user, &other_identity, group.id)
            .await
            .is_err()
    );
    let observation = causal_adoption::observe_group(&db, &user, &identity, group.id)
        .await
        .unwrap();
    sqlx::query("INSERT INTO models_group (group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES ('duplicate','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(&group.unique_global_id).execute(&db).await.unwrap();
    assert!(
        causal_adoption::enroll_observed_group(&db, &user, &identity, &observation)
            .await
            .is_err()
    );
    let nonempty = common::legacy_group(&db, &user, "nonempty").await;
    let model = common::legacy_model(&db, &user, "member").await;
    group_db::set_group_id_on_models(&db, &user, Some(nonempty.id), vec![model], None)
        .await
        .unwrap();
    assert!(
        causal_adoption::observe_group(&db, &user, &identity, nonempty.id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn failed_observation_enrollment_rolls_back_birth_and_event_allocation() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = common::legacy_group(&db, &user, "legacy").await;
    let observation = causal_adoption::observe_group(&db, &user, &identity, group.id)
        .await
        .unwrap();
    let before:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM entity_births),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_observation_binding BEFORE INSERT ON causal_genesis_bindings BEGIN SELECT RAISE(ABORT,'injected observation failure'); END").execute(&db).await.unwrap();
    let error = causal_adoption::enroll_observed_group(&db, &user, &identity, &observation)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("injected observation failure"));
    let after:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM entity_births),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
    assert_eq!(after, before);
    let birth: Option<String> =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id=?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(birth, None);
}

#[tokio::test]
async fn install_rejects_occupied_identity_and_rolls_back_injected_failure() {
    let (_ad, a) = common::test_db().await;
    let (_bd, b) = common::test_db().await;
    let (_cd, c) = common::test_db().await;
    let user = User::default();
    let ai = user_db::get_sync_identity(&a, user.id)
        .await
        .unwrap()
        .unwrap();
    let bi = user_db::get_sync_identity(&b, user.id)
        .await
        .unwrap()
        .unwrap();
    let ci = user_db::get_sync_identity(&c, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = common::legacy_group(&a, &user, "source").await;
    let observation = causal_adoption::observe_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let bundle = causal_adoption::enroll_observed_group(&a, &user, &ai, &observation)
        .await
        .unwrap();
    sqlx::query("INSERT INTO models_group (group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES ('occupied','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(&bundle.entity().global_id).execute(&b).await.unwrap();
    let peer = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    assert!(
        causal_adoption::install_observed_group(&b, &user, &bi, &peer, &bundle)
            .await
            .is_err(),
        "existing target identity must not be auto-adopted"
    );
    let name: String = sqlx::query_scalar(
        "SELECT group_name FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(user.id)
    .bind(&bundle.entity().global_id)
    .fetch_one(&b)
    .await
    .unwrap();
    assert_eq!(name, "occupied");
    let peer = sync_peer::bind_peer(&c, &user, &ci, &ai).await.unwrap();
    let before:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM entity_births)").fetch_one(&c).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_observation_install BEFORE INSERT ON causal_genesis_bindings BEGIN SELECT RAISE(ABORT,'injected install failure'); END").execute(&c).await.unwrap();
    let error = causal_adoption::install_observed_group(&c, &user, &ci, &peer, &bundle)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("injected install failure"));
    let after:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM entity_births)").fetch_one(&c).await.unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn observation_rejects_membership_add_remove_aba_even_when_empty_again() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = common::legacy_group(&db, &user, "empty").await;
    let model = common::legacy_model(&db, &user, "member").await;
    let observation = causal_adoption::observe_group(&db, &user, &identity, group.id)
        .await
        .unwrap();
    group_db::set_group_id_on_models(
        &db,
        &user,
        Some(group.id),
        vec![model],
        Some("2001-01-01T00:00:00Z"),
    )
    .await
    .unwrap();
    group_db::set_group_id_on_models(&db, &user, None, vec![model], Some("2001-01-01T00:00:00Z"))
        .await
        .unwrap();
    assert!(
        causal_adoption::enroll_observed_group(&db, &user, &identity, &observation)
            .await
            .is_err(),
        "returning to an empty folder does not restore the observed state"
    );
}

#[tokio::test]
async fn edited_known_birth_is_preserved_through_observation_and_install() {
    let (_ad, a) = common::test_db().await;
    let (_bd, b) = common::test_db().await;
    let user = User::default();
    let ai = user_db::get_sync_identity(&a, user.id)
        .await
        .unwrap()
        .unwrap();
    let bi = user_db::get_sync_identity(&b, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&a, &user, "before", None)
        .await
        .unwrap();
    let birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id=?")
            .bind(group.id)
            .fetch_one(&a)
            .await
            .unwrap();
    group_db::edit_group(&a, &user, group.id, "edited", None, None)
        .await
        .unwrap();
    let observation = causal_adoption::observe_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let bundle = causal_adoption::enroll_observed_group(&a, &user, &ai, &observation)
        .await
        .unwrap();
    assert_eq!(bundle.entity().birth_id.as_deref(), Some(birth.as_str()));
    assert!(matches!(
        bundle.envelope().action,
        Action::LegacyObserved { .. }
    ));
    let peer = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let installed = causal_adoption::install_observed_group(&b, &user, &bi, &peer, &bundle)
        .await
        .unwrap();
    let target_birth: Option<String> =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id=?")
            .bind(installed.local_id)
            .fetch_one(&b)
            .await
            .unwrap();
    assert_eq!(target_birth, Some(birth));
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM local_creation_records WHERE user_id=? AND entity_kind='group'",
    )
    .bind(user.id)
    .fetch_one(&a)
    .await
    .unwrap();
    assert_eq!(
        count, 1,
        "the earlier authentic creation evidence must not be replaced or duplicated"
    );
}

#[tokio::test]
async fn observed_root_preserves_concurrent_names_as_choices() {
    let (_ad, a) = common::test_db().await;
    let (_bd, b) = common::test_db().await;
    let user = User::default();
    let ai = user_db::get_sync_identity(&a, user.id)
        .await
        .unwrap()
        .unwrap();
    let bi = user_db::get_sync_identity(&b, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = common::legacy_group(&a, &user, "old").await;
    group_db::edit_group(&a, &user, group.id, "observed", None, None)
        .await
        .unwrap();
    let observation = causal_adoption::observe_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let root = causal_adoption::enroll_observed_group(&a, &user, &ai, &observation)
        .await
        .unwrap();
    let ba = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let target = causal_adoption::install_observed_group(&b, &user, &bi, &ba, &root)
        .await
        .unwrap()
        .local_id;
    group_db::edit_group(&a, &user, group.id, "alpha", None, None)
        .await
        .unwrap();
    group_db::edit_group(&b, &user, target, "bravo", None, None)
        .await
        .unwrap();
    let bstate = causal_group_lifecycle::get_state(&b, &user, &bi, target)
        .await
        .unwrap();
    let dot = match bstate.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready {x:?}"),
    };
    let event = causal_group_lifecycle::export_operation(&b, &user, &bi, &dot)
        .await
        .unwrap();
    let ab = sync_peer::bind_peer(&a, &user, &ai, &bi).await.unwrap();
    let result = causal_group_lifecycle::import_operation(&a, &user, &ai, &ab, &event)
        .await
        .unwrap();
    let token = match result {
        OperationImportResult::Applied(state) => match state.name {
            NameState::Conflict { choices, token } => {
                assert!(choices.iter().any(|choice| choice.name == "alpha"));
                assert!(choices.iter().any(|choice| choice.name == "bravo"));
                token
            }
            x => panic!("expected name conflict {x:?}"),
        },
        x @ OperationImportResult::NeedsDependencies { .. } => panic!("expected applied {x:?}"),
    };
    let result = db::causal_group_rename::resolve(&a, &user, &ai, group.id, &token, "bravo")
        .await
        .unwrap();
    assert!(
        matches!(result,db::causal_group_rename::ResolveResult::Resolved(NameState::Ready{ref name,..}) if name=="bravo")
    );
}

#[tokio::test]
async fn observed_root_concurrent_delete_and_rename_keeps_group() {
    let (_ad, a) = common::test_db().await;
    let (_bd, b) = common::test_db().await;
    let user = User::default();
    let ai = user_db::get_sync_identity(&a, user.id)
        .await
        .unwrap()
        .unwrap();
    let bi = user_db::get_sync_identity(&b, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = common::legacy_group(&a, &user, "old").await;
    group_db::edit_group(&a, &user, group.id, "observed", None, None)
        .await
        .unwrap();
    let observation = causal_adoption::observe_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let root = causal_adoption::enroll_observed_group(&a, &user, &ai, &observation)
        .await
        .unwrap();
    let ba = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let target = causal_adoption::install_observed_group(&b, &user, &bi, &ba, &root)
        .await
        .unwrap()
        .local_id;
    group_db::delete_group(&a, &user, group.id).await.unwrap();
    group_db::edit_group(&b, &user, target, "offline", None, None)
        .await
        .unwrap();
    let bstate = causal_group_lifecycle::get_state(&b, &user, &bi, target)
        .await
        .unwrap();
    let dot = match bstate.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        x => panic!("expected ready {x:?}"),
    };
    let rename = causal_group_lifecycle::export_operation(&b, &user, &bi, &dot)
        .await
        .unwrap();
    let ab = sync_peer::bind_peer(&a, &user, &ai, &bi).await.unwrap();
    let result = causal_group_lifecycle::import_operation(&a, &user, &ai, &ab, &rename)
        .await
        .unwrap();
    assert!(
        matches!(result,OperationImportResult::Applied(ref state) if matches!(state.lifecycle,causal_group_lifecycle::GroupLifecycle::Active))
    );
    let row: (String, Option<String>) =
        sqlx::query_as("SELECT group_name,group_deleted_at FROM models_group WHERE group_id=?")
            .bind(group.id)
            .fetch_one(&a)
            .await
            .unwrap();
    assert_eq!(row, ("offline".into(), None));
}

#[tokio::test]
async fn observation_enrollment_retry_rejects_changes_after_success() {
    for change in ["rename", "member", "delete", "recreate"] {
        let (_dir, db) = common::test_db().await;
        let user = User::default();
        let identity = user_db::get_sync_identity(&db, user.id)
            .await
            .unwrap()
            .unwrap();
        let group = common::legacy_group(&db, &user, "original").await;
        let observation = causal_adoption::observe_group(&db, &user, &identity, group.id)
            .await
            .unwrap();
        let first = causal_adoption::enroll_observed_group(&db, &user, &identity, &observation)
            .await
            .unwrap();
        let retry = causal_adoption::enroll_observed_group(&db, &user, &identity, &observation)
            .await
            .unwrap();
        assert_eq!(retry.event(), first.event());
        match change {
            "rename" => group_db::edit_group(&db, &user, group.id, "changed", None, None)
                .await
                .unwrap(),
            "member" => {
                let model = common::legacy_model(&db, &user, "member").await;
                group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
                    .await
                    .unwrap();
            }
            "delete" => group_db::delete_group(&db, &user, group.id).await.unwrap(),
            "recreate" => {
                group_db::delete_group(&db, &user, group.id).await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
                assert_eq!(group_db::purge_trashed_groups(&db, 0).await.unwrap(), 1);
                sqlx::query("INSERT INTO models_group (group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES ('replacement','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(&group.unique_global_id).execute(&db).await.unwrap();
            }
            _ => unreachable!(),
        }
        let before: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings), (SELECT COUNT(*) FROM causal_events), (SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
        assert!(
            causal_adoption::enroll_observed_group(&db, &user, &identity, &observation)
                .await
                .is_err(),
            "stale retry after {change} must not return an outdated bundle"
        );
        let after: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings), (SELECT COUNT(*) FROM causal_events), (SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
        assert_eq!(after, before);
    }
}

#[tokio::test]
async fn observation_evidence_counts_toward_combined_byte_limit() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let name = "x".repeat(db::causal_envelope::MAX_NAME_BYTES);
    let mut previous_bytes = 0_i64;
    for _ in 0..200 {
        let group = common::legacy_group(&db, &user, &name).await;
        let observation = causal_adoption::observe_group(&db, &user, &identity, group.id)
            .await
            .unwrap();
        let before: (i64, i64, i64) = sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(length(CAST(e.envelope AS BLOB))+length(CAST(b.materialization AS BLOB))+length(CAST(b.local_guard AS BLOB))+length(CAST(COALESCE(b.observation_evidence,'') AS BLOB))),0), (SELECT counter FROM group_causal_replica WHERE singleton=1) FROM causal_genesis_bindings b JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.capability='group_lifecycle_v1'").bind(user.id).fetch_one(&db).await.unwrap();
        let contribution = before.1 - previous_bytes;
        let result =
            causal_adoption::enroll_observed_group(&db, &user, &identity, &observation).await;
        if contribution > 0
            && before.1 + contribution
                > i64::try_from(db::causal_store::MAX_OWNER_BYTES).unwrap() + 1024
        {
            assert!(
                result.is_err(),
                "observation evidence must count toward the aggregate byte limit"
            );
            let after: (i64, i64, i64) = sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(length(CAST(e.envelope AS BLOB))+length(CAST(b.materialization AS BLOB))+length(CAST(b.local_guard AS BLOB))+length(CAST(COALESCE(b.observation_evidence,'') AS BLOB))),0), (SELECT counter FROM group_causal_replica WHERE singleton=1) FROM causal_genesis_bindings b JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.capability='group_lifecycle_v1'").bind(user.id).fetch_one(&db).await.unwrap();
            assert_eq!(after, before);

            return;
        }
        result.unwrap();
        previous_bytes = before.1;
    }
    panic!("test did not reach the production byte limit");
}
