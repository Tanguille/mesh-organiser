//! Populated observations preserve a snapshot while leaving earlier ancestry unknown.

use db::{
    blob_db,
    causal_adoption::{self, ModelBlobBinding},
    causal_envelope::Action,
    causal_group_lifecycle::{self, GroupLifecycle, OperationImportResult},
    causal_group_rename::NameState,
    group_db,
    model::user::User,
    model_db, random_hex_32, sync_peer, user_db,
};

mod common;

async fn assert_failed_installs_roll_back(
    db: &db::db_context::DbContext,
    user: &User,
    identity: &user_db::SyncIdentity,
    peer: &sync_peer::PeerBinding,
    bundle: &causal_adoption::ObservedPopulatedGroupBundle,
    mappings: &[ModelBlobBinding],
) {
    let before: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group), (SELECT COUNT(*) FROM models), (SELECT COUNT(*) FROM causal_genesis_bindings)").fetch_one(db).await.unwrap();
    assert!(
        causal_adoption::install_observed_populated_group(
            db,
            user,
            identity,
            peer,
            bundle,
            &mappings[..1]
        )
        .await
        .is_err()
    );
    let after: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group), (SELECT COUNT(*) FROM models), (SELECT COUNT(*) FROM causal_genesis_bindings)").fetch_one(db).await.unwrap();
    assert_eq!(
        after, before,
        "missing blob mapping must roll back the whole install"
    );
    sqlx::query("CREATE TRIGGER reject_observed_binding BEFORE INSERT ON causal_genesis_bindings BEGIN SELECT RAISE(ABORT,'injected binding failure'); END").execute(db).await.unwrap();
    assert!(
        causal_adoption::install_observed_populated_group(
            db, user, identity, peer, bundle, mappings
        )
        .await
        .is_err()
    );
    sqlx::query("DROP TRIGGER reject_observed_binding")
        .execute(db)
        .await
        .unwrap();
    let after_failure: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group), (SELECT COUNT(*) FROM models), (SELECT COUNT(*) FROM causal_genesis_bindings)").fetch_one(db).await.unwrap();
    assert_eq!(
        after_failure, before,
        "binding failure must roll back all installed rows"
    );
}

fn assert_observation_roots(bundle: &causal_adoption::ObservedPopulatedGroupBundle) {
    assert!(matches!(
        bundle.group_envelope().action,
        Action::LegacyObserved { .. }
    ));
    assert_eq!(bundle.model_envelopes().len(), 2);
    for event in bundle.model_envelopes() {
        assert!(matches!(event.action, Action::LegacyObserved { .. }));
        assert_eq!(event.parents, [] as [db::causal_envelope::Dot; 0]);
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn populated_observation_installs_all_members_without_fabricating_history() {
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
    let group = common::legacy_group(&a, &user, "group").await;
    let hash1 = random_hex_32();
    let hash2 = random_hex_32();
    let blob1 = blob_db::add_blob(&a, &hash1, "stl", 12, None)
        .await
        .unwrap();
    let blob2 = blob_db::add_blob(&a, &hash2, "obj", 34, None)
        .await
        .unwrap();
    let model1 = model_db::add_model(&a, &user, "known", blob1, None, None)
        .await
        .unwrap();
    let model2 = sqlx::query("INSERT INTO models (model_name,model_blob_id,model_added,model_user_id,model_unique_global_id,model_last_modified) VALUES ('legacy',?,'2020-01-01T00:00:00Z',?,?,'2020-01-01T00:00:00Z')").bind(blob2).bind(user.id).bind(random_hex_32()).execute(&a).await.unwrap().last_insert_rowid();
    sqlx::query("UPDATE models SET model_url='https://example.test/model',model_desc='edited description',model_flags=3 WHERE model_id=?").bind(model1).execute(&a).await.unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(group.id), vec![model1, model2], None)
        .await
        .unwrap();
    let history_before: Vec<String> = sqlx::query_scalar("SELECT envelope FROM causal_events WHERE encoding='local_history_v1' ORDER BY writer_id,counter").fetch_all(&a).await.unwrap();
    let capture = causal_adoption::observe_populated_group(&a, &user, &ai, group.id).await;
    assert!(
        capture.is_ok(),
        "active edited members should be observable: {capture:?}"
    );
    let capture = capture.unwrap();
    let bundle = causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture)
        .await
        .unwrap();
    let source_retry = causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture)
        .await
        .unwrap();
    assert_eq!(source_retry.group_event(), bundle.group_event());
    assert_observation_roots(&bundle);
    assert_eq!(history_before, sqlx::query_scalar::<_, String>("SELECT envelope FROM causal_events WHERE encoding='local_history_v1' ORDER BY writer_id,counter").fetch_all(&a).await.unwrap());
    let target_blob1 = blob_db::add_blob(&b, &hash1, "stl", 12, None)
        .await
        .unwrap();
    let target_blob2 = blob_db::add_blob(&b, &hash2, "obj", 34, None)
        .await
        .unwrap();
    let global1: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id=?")
            .bind(model1)
            .fetch_one(&a)
            .await
            .unwrap();
    let mappings: Vec<_> = bundle
        .model_envelopes()
        .iter()
        .map(|event| ModelBlobBinding {
            entity: event.entity.clone(),
            local_blob_id: if event.entity.global_id == global1 {
                target_blob1
            } else {
                target_blob2
            },
        })
        .collect();
    let peer = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    assert_failed_installs_roll_back(&b, &user, &bi, &peer, &bundle, &mappings).await;
    let installed = causal_adoption::install_observed_populated_group(
        &b, &user, &bi, &peer, &bundle, &mappings,
    )
    .await
    .unwrap();
    assert!(!installed.already_present);
    assert_eq!(installed.models.len(), 2);
    let retry = causal_adoption::install_observed_populated_group(
        &b, &user, &bi, &peer, &bundle, &mappings,
    )
    .await
    .unwrap();
    assert!(retry.already_present);
    assert_eq!(retry.group_id, installed.group_id);
    assert_eq!(retry.models, installed.models);
    let members: Vec<(String,Option<String>,Option<String>,i64)> = sqlx::query_as("SELECT model_name,model_url,model_desc,model_flags FROM models WHERE model_user_id=? AND model_group_id=? ORDER BY model_name").bind(user.id).bind(installed.group_id).fetch_all(&b).await.unwrap();
    assert_eq!(
        members,
        vec![
            (
                "known".into(),
                Some("https://example.test/model".into()),
                Some("edited description".into()),
                3
            ),
            ("legacy".into(), None, None, 0)
        ]
    );
    let synthetic: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events WHERE user_id=? AND encoding='portable_v2' AND json_extract(envelope,'$.action.type') IN ('model_moved','group_member_added')").bind(user.id).fetch_one(&b).await.unwrap();
    assert_eq!(
        synthetic, 0,
        "observed membership is evidence, not a fabricated user action"
    );
    sqlx::query("UPDATE models SET model_desc='later edit' WHERE model_id=?")
        .bind(model1)
        .execute(&a)
        .await
        .unwrap();
    let before_retry: (i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&a).await.unwrap();
    assert!(
        causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture)
            .await
            .is_err()
    );
    let after_retry: (i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&a).await.unwrap();
    assert_eq!(after_retry, before_retry);
}

#[tokio::test]
async fn populated_capture_rejects_metadata_membership_and_blob_aba() {
    for change in ["metadata", "membership", "blob", "label"] {
        let (_dir, db) = common::test_db().await;
        let user = User::default();
        let identity = user_db::get_sync_identity(&db, user.id)
            .await
            .unwrap()
            .unwrap();
        let group = common::legacy_group(&db, &user, "group").await;
        let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 12, None)
            .await
            .unwrap();
        let model = model_db::add_model(&db, &user, "part", blob, None, None)
            .await
            .unwrap();
        group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
            .await
            .unwrap();
        let capture = causal_adoption::observe_populated_group(&db, &user, &identity, group.id)
            .await
            .unwrap();
        match change {
            "metadata" => {
                sqlx::query("UPDATE models SET model_desc='changed' WHERE model_id=?")
                    .bind(model)
                    .execute(&db)
                    .await
                    .unwrap();
                sqlx::query("UPDATE models SET model_desc=NULL WHERE model_id=?")
                    .bind(model)
                    .execute(&db)
                    .await
                    .unwrap();
            }
            "membership" => {
                group_db::set_group_id_on_models(&db, &user, None, vec![model], None)
                    .await
                    .unwrap();
                group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
                    .await
                    .unwrap();
            }
            "blob" => {
                sqlx::query("UPDATE blobs SET blob_size=13 WHERE blob_id=?")
                    .bind(blob)
                    .execute(&db)
                    .await
                    .unwrap();
                sqlx::query("UPDATE blobs SET blob_size=12 WHERE blob_id=?")
                    .bind(blob)
                    .execute(&db)
                    .await
                    .unwrap();
            }
            "label" => {
                let label = db::label_db::add_label(&db, &user, "label", 0, None)
                    .await
                    .unwrap();
                db::label_db::add_labels_on_models(&db, &user, &[label.id], &[model], None)
                    .await
                    .unwrap();
                db::label_db::remove_labels_from_models(&db, &user, &[label.id], &[model], None)
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let before: (i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
        assert!(
            causal_adoption::enroll_observed_populated_group(&db, &user, &identity, &capture)
                .await
                .is_err(),
            "{change} ABA must invalidate the whole capture"
        );
        let after: (i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
        assert_eq!(after, before);
    }
}

#[tokio::test]
async fn populated_observation_scopes_trashed_members_to_the_owner() {
    {
        let (_dir, db) = common::test_db().await;
        let user = User::default();
        let identity = user_db::get_sync_identity(&db, user.id)
            .await
            .unwrap()
            .unwrap();
        let group = common::legacy_group(&db, &user, "group").await;
        let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 12, None)
            .await
            .unwrap();
        let model = model_db::add_model(&db, &user, "part", blob, None, None)
            .await
            .unwrap();
        group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
            .await
            .unwrap();
        let other_id = user_db::add_user(
            &db,
            "other",
            "other-populated@example.test",
            &random_hex_32(),
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
        assert!(
            causal_adoption::observe_populated_group(&db, &user, &other_identity, group.id)
                .await
                .is_err()
        );
        assert!(
            causal_adoption::observe_populated_group(&db, &other, &other_identity, group.id)
                .await
                .is_err()
        );
        model_db::trash_models(&db, &user, &[model]).await.unwrap();
        let before: (i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
        assert!(
            causal_adoption::observe_populated_group(&db, &user, &identity, group.id)
                .await
                .is_ok(),
            "an owned trashed member must be observable"
        );
        let after: (i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
        assert_eq!(after, before);
    }
}

#[tokio::test]
async fn populated_enrollment_failure_rolls_back_all_births_and_counters() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = common::legacy_group(&db, &user, "group").await;
    let model1 = common::legacy_model(&db, &user, &random_hex_32()).await;
    let model2 = common::legacy_model(&db, &user, &random_hex_32()).await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model1, model2], None)
        .await
        .unwrap();
    let capture = causal_adoption::observe_populated_group(&db, &user, &identity, group.id)
        .await
        .unwrap();
    let before: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM entity_births),(SELECT COUNT(*) FROM causal_events),(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM group_birth_adoption_permits)").fetch_one(&db).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_second_observation_binding BEFORE INSERT ON causal_genesis_bindings WHEN (SELECT COUNT(*) FROM causal_genesis_bindings)>=1 BEGIN SELECT RAISE(ABORT,'injected second binding failure'); END").execute(&db).await.unwrap();
    assert!(
        causal_adoption::enroll_observed_populated_group(&db, &user, &identity, &capture)
            .await
            .is_err()
    );
    sqlx::query("DROP TRIGGER reject_second_observation_binding")
        .execute(&db)
        .await
        .unwrap();
    let after: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM entity_births),(SELECT COUNT(*) FROM causal_events),(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT COUNT(*) FROM group_birth_adoption_permits)").fetch_one(&db).await.unwrap();
    assert_eq!(after, before);
    let births: (Option<String>,Option<String>,Option<String>) = sqlx::query_as("SELECT (SELECT group_birth_id FROM models_group WHERE group_id=?),(SELECT model_birth_id FROM models WHERE model_id=?),(SELECT model_birth_id FROM models WHERE model_id=?)").bind(group.id).bind(model1).bind(model2).fetch_one(&db).await.unwrap();
    assert_eq!(births, (None, None, None));
}

#[tokio::test]
async fn populated_manifest_bytes_count_toward_storage_limit() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let limit = i64::try_from(db::causal_store::MAX_OWNER_BYTES).unwrap();
    let mut last_core = 0_i64;
    let mut previous_core = 0_i64;
    for _ in 0..200 {
        let (core, manifests): (i64, i64) = sqlx::query_as("SELECT COALESCE((SELECT SUM(length(CAST(e.envelope AS BLOB))+length(CAST(b.materialization AS BLOB))+length(CAST(b.local_guard AS BLOB))+COALESCE(length(CAST(b.observation_evidence AS BLOB)),0)) FROM causal_genesis_bindings b JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.capability='group_lifecycle_v1'),0), COALESCE((SELECT SUM(length(CAST(manifest AS BLOB))) FROM causal_observation_batches WHERE user_id=?),0)").bind(user.id).bind(user.id).fetch_one(&db).await.unwrap();
        let remaining = limit - core;
        let base = (last_core - previous_core) - 120_000;
        let candidate = (remaining - base - manifests / 2) / 2;
        let is_final = last_core > previous_core
            && candidate > 0
            && (candidate <= i64::try_from(db::causal_envelope::MAX_NAME_BYTES).unwrap()
                || remaining - base - 120_000 < manifests - 1024);
        let name_len = if is_final {
            usize::try_from(candidate.min(60_000)).unwrap()
        } else {
            60_000
        };
        assert!(name_len <= db::causal_envelope::MAX_NAME_BYTES);
        let group = common::legacy_group(&db, &user, &"g".repeat(name_len)).await;
        let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 1, None)
            .await
            .unwrap();
        let model = model_db::add_model(&db, &user, "model", blob, None, None)
            .await
            .unwrap();
        group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
            .await
            .unwrap();
        let capture = causal_adoption::observe_populated_group(&db, &user, &identity, group.id)
            .await
            .unwrap();
        let result =
            causal_adoption::enroll_observed_populated_group(&db, &user, &identity, &capture).await;
        if is_final {
            assert!(
                result.is_err(),
                "all previously stored manifest bytes must count toward the aggregate limit"
            );

            return;
        }
        result.unwrap_or_else(|error| panic!("unexpected rejection: {error:?}; core={core}, manifests={manifests}, candidate={candidate}, base={base}"));
        previous_core = core;
        let (next_core,): (i64,) = sqlx::query_as("SELECT COALESCE(SUM(length(CAST(e.envelope AS BLOB))+length(CAST(b.materialization AS BLOB))+length(CAST(b.local_guard AS BLOB))+COALESCE(length(CAST(b.observation_evidence AS BLOB)),0)),0) FROM causal_genesis_bindings b JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.capability='group_lifecycle_v1'").bind(user.id).fetch_one(&db).await.unwrap();
        last_core = next_core;
    }
    panic!("production byte limit was not reached");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn populated_observation_preserves_three_device_rename_conflict_and_concurrent_delete() {
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
    let group = common::legacy_group(&a, &user, "observed").await;
    let hash = random_hex_32();
    let blob = blob_db::add_blob(&a, &hash, "stl", 12, None).await.unwrap();
    let model = model_db::add_model(&a, &user, "member", blob, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    let capture = causal_adoption::observe_populated_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let bundle = causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture)
        .await
        .unwrap();
    let mut local_groups = Vec::new();
    for (db, identity) in [(&b, &bi), (&c, &ci)] {
        let target_blob = blob_db::add_blob(db, &hash, "stl", 12, None).await.unwrap();
        let peer = sync_peer::bind_peer(db, &user, identity, &ai)
            .await
            .unwrap();
        let mappings = [ModelBlobBinding {
            entity: bundle.model_envelopes()[0].entity.clone(),
            local_blob_id: target_blob,
        }];
        local_groups.push(
            causal_adoption::install_observed_populated_group(
                db, &user, identity, &peer, &bundle, &mappings,
            )
            .await
            .unwrap()
            .group_id,
        );
    }
    let ab = sync_peer::bind_peer(&a, &user, &ai, &bi).await.unwrap();
    let ac = sync_peer::bind_peer(&a, &user, &ai, &ci).await.unwrap();
    for (db, identity, group_id, name, peer) in [
        (&b, &bi, local_groups[0], "bravo", &ab),
        (&c, &ci, local_groups[1], "charlie", &ac),
    ] {
        group_db::edit_group(db, &user, group_id, name, None, None)
            .await
            .unwrap();
        let state = causal_group_lifecycle::get_state(db, &user, identity, group_id)
            .await
            .unwrap();
        let dot = match state.name {
            NameState::Ready { token, .. } => token.frontier[0].clone(),
            other => panic!("expected ready: {other:?}"),
        };
        let op = causal_group_lifecycle::export_operation(db, &user, identity, &dot)
            .await
            .unwrap();
        assert!(matches!(
            causal_group_lifecycle::import_operation(&a, &user, &ai, peer, &op)
                .await
                .unwrap(),
            OperationImportResult::Applied(_)
        ));
    }
    let state = causal_group_lifecycle::get_state(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let token = match state.name {
        NameState::Conflict { choices, token } => {
            assert!(choices.iter().any(|choice| choice.name == "bravo"));
            assert!(choices.iter().any(|choice| choice.name == "charlie"));
            token
        }
        other => panic!("expected concurrent choices: {other:?}"),
    };
    assert!(matches!(
        db::causal_group_rename::resolve(&a, &user, &ai, group.id, &token, "bravo")
            .await
            .unwrap(),
        db::causal_group_rename::ResolveResult::Resolved(_)
    ));
    group_db::delete_group(&c, &user, local_groups[1])
        .await
        .unwrap();
    let state = causal_group_lifecycle::get_state(&c, &user, &ci, local_groups[1])
        .await
        .unwrap();
    let deletion = match state.lifecycle {
        GroupLifecycle::Deleted { deletions } => deletions[0].clone(),
        other @ GroupLifecycle::Active => panic!("expected deletion: {other:?}"),
    };
    let operation = causal_group_lifecycle::export_operation(&c, &user, &ci, &deletion)
        .await
        .unwrap();
    let result = causal_group_lifecycle::import_operation(&a, &user, &ai, &ac, &operation)
        .await
        .unwrap();
    assert!(
        matches!(result, OperationImportResult::Applied(ref state) if matches!(state.lifecycle, GroupLifecycle::Active))
    );
    let row: (String, Option<String>, Option<i64>) = sqlx::query_as("SELECT g.group_name,g.group_deleted_at,m.model_group_id FROM models_group g JOIN models m ON m.model_id=? WHERE g.group_id=?").bind(model).bind(group.id).fetch_one(&a).await.unwrap();
    assert_eq!(row, ("bravo".into(), None, Some(group.id)));
}

#[tokio::test]
async fn observed_member_can_move_away_and_replicate_original_move() {
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
    let group = common::legacy_group(&a, &user, "group").await;
    let hash = random_hex_32();
    let blob_a = blob_db::add_blob(&a, &hash, "stl", 12, None).await.unwrap();
    let model = model_db::add_model(&a, &user, "member", blob_a, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    let capture = causal_adoption::observe_populated_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let bundle = causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture)
        .await
        .unwrap();
    let blob_b = blob_db::add_blob(&b, &hash, "stl", 12, None).await.unwrap();
    let peer_b = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let mappings = [ModelBlobBinding {
        entity: bundle.model_envelopes()[0].entity.clone(),
        local_blob_id: blob_b,
    }];
    let installed = causal_adoption::install_observed_populated_group(
        &b, &user, &bi, &peer_b, &bundle, &mappings,
    )
    .await
    .unwrap();
    let target_model = installed.models[0].1;
    group_db::set_group_id_on_models(&b, &user, None, vec![target_model], None)
        .await
        .unwrap();
    let state = db::causal_model_move::get_state(&b, &user, &bi, target_model)
        .await
        .unwrap();
    let dot = match state {
        db::causal_model_move::MoveState::Ready {
            destination: None,
            token,
        } => token.frontier[0].clone(),
        other => panic!("expected ungrouped move: {other:?}"),
    };
    let operation = causal_group_lifecycle::export_operation(&b, &user, &bi, &dot)
        .await
        .unwrap();
    assert!(operation.envelopes().iter().any(|event| matches!(&event.action, Action::ModelMoved { from: Some(from), to: None, .. } if from==bundle.group_entity())));
    let peer_a = sync_peer::bind_peer(&a, &user, &ai, &bi).await.unwrap();
    assert!(matches!(
        causal_group_lifecycle::import_operation(&a, &user, &ai, &peer_a, &operation)
            .await
            .unwrap(),
        OperationImportResult::Applied(_)
    ));
    let destination: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(model)
            .fetch_one(&a)
            .await
            .unwrap();
    assert_eq!(destination, None);
}
