//! Trash timestamps are observed state, not invented lifecycle history.
use db::{
    blob_db,
    causal_adoption::{self, ModelBlobBinding},
    group_db, label_db,
    model::user::User,
    model_db, random_hex_32, resource_db, sync_peer, user_db,
};

mod common;

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn mixed_trashed_group_preserves_project_labels_and_exact_deletion_time() {
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
    let project = resource_db::add_resource(&a, &user, "project", None)
        .await
        .unwrap();
    let hash1 = random_hex_32();
    let hash2 = random_hex_32();
    let blob1 = blob_db::add_blob(&a, &hash1, "stl", 12, None)
        .await
        .unwrap();
    let blob2 = blob_db::add_blob(&a, &hash2, "obj", 34, None)
        .await
        .unwrap();
    let live = model_db::add_model(&a, &user, "live", blob1, None, None)
        .await
        .unwrap();
    let trashed = model_db::add_model(&a, &user, "trashed", blob2, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(group.id), vec![live, trashed], None)
        .await
        .unwrap();
    resource_db::set_resource_on_group(&a, &user, Some(project.id), group.id, None)
        .await
        .unwrap();
    let label = label_db::add_label(&a, &user, "label", 13, None)
        .await
        .unwrap();
    label_db::add_labels_on_models(&a, &user, &[label.id], &[trashed], None)
        .await
        .unwrap();
    model_db::trash_models(&a, &user, &[trashed]).await.unwrap();
    let deleted: String = sqlx::query_scalar("SELECT model_deleted FROM models WHERE model_id=?")
        .bind(trashed)
        .fetch_one(&a)
        .await
        .unwrap();
    let capture = causal_adoption::observe_populated_group(&a, &user, &ai, group.id).await;
    assert!(
        capture.is_ok(),
        "group containing a trashed member should be observable: {capture:?}"
    );
    let bundle =
        causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture.unwrap())
            .await
            .unwrap();
    let target_blob1 = blob_db::add_blob(&b, &hash1, "stl", 12, None)
        .await
        .unwrap();
    let target_blob2 = blob_db::add_blob(&b, &hash2, "obj", 34, None)
        .await
        .unwrap();
    let live_global: String =
        sqlx::query_scalar("SELECT model_unique_global_id FROM models WHERE model_id=?")
            .bind(live)
            .fetch_one(&a)
            .await
            .unwrap();
    let mapping: Vec<_> = bundle
        .model_envelopes()
        .iter()
        .map(|event| ModelBlobBinding {
            entity: event.entity.clone(),
            local_blob_id: if event.entity.global_id == live_global {
                target_blob1
            } else {
                target_blob2
            },
        })
        .collect();
    let peer = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let installed =
        causal_adoption::install_observed_populated_group(&b, &user, &bi, &peer, &bundle, &mapping)
            .await
            .unwrap();
    let retry =
        causal_adoption::install_observed_populated_group(&b, &user, &bi, &peer, &bundle, &mapping)
            .await
            .unwrap();
    assert!(retry.already_present);
    assert_eq!(retry.models, installed.models);
    let rows: Vec<(i64,String,Option<String>)> = sqlx::query_as("SELECT model_id,model_name,model_deleted FROM models WHERE model_group_id=? ORDER BY model_name").bind(installed.group_id).fetch_all(&b).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].1.as_str(), rows[0].2.as_ref()), ("live", None));
    assert_eq!(
        (rows[1].1.as_str(), rows[1].2.as_deref()),
        ("trashed", Some(deleted.as_str()))
    );
    let project_name: Option<String> = sqlx::query_scalar("SELECT r.resource_name FROM models_group g JOIN resources r ON r.resource_id=g.group_resource_id WHERE g.group_id=?").bind(installed.group_id).fetch_optional(&b).await.unwrap();
    assert_eq!(project_name.as_deref(), Some("project"));
    let label_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models_labels ml JOIN labels l ON l.label_id=ml.label_id WHERE ml.model_id=? AND l.label_name='label'").bind(rows[1].0).fetch_one(&b).await.unwrap();
    assert_eq!(label_count, 1);
    let fabricated: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events WHERE encoding='portable_v2' AND json_extract(envelope,'$.action.type') IN ('model_moved','model_detached','group_member_added')").fetch_one(&b).await.unwrap();
    assert_eq!(fabricated, 0);
    group_db::delete_group(&b, &user, installed.group_id)
        .await
        .unwrap();
    let restore = common::group_restore_request(&b, &user, installed.group_id).await;
    group_db::restore_groups(&b, &user, &[restore], 30)
        .await
        .unwrap();
    let still_trashed: (Option<i64>, String) =
        sqlx::query_as("SELECT model_group_id,model_deleted FROM models WHERE model_id=?")
            .bind(rows[1].0)
            .fetch_one(&b)
            .await
            .unwrap();
    assert_eq!(
        still_trashed,
        (Some(installed.group_id), deleted.clone()),
        "group restore must preserve the model Trash timestamp"
    );
    model_db::restore_models(&b, &user, &[rows[1].0])
        .await
        .unwrap();
    let state = db::causal_group_lifecycle::get_state(&b, &user, &bi, installed.group_id)
        .await
        .unwrap();
    assert!(matches!(
        state.name,
        db::causal_group_rename::NameState::Paused { .. }
    ));
    let restored: (Option<i64>, Option<String>) =
        sqlx::query_as("SELECT model_group_id,model_deleted FROM models WHERE model_id=?")
            .bind(rows[1].0)
            .fetch_one(&b)
            .await
            .unwrap();
    assert_eq!(restored, (Some(installed.group_id), None));
}

#[tokio::test]
async fn all_trashed_group_preserves_project_after_target_purge() {
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
    let project = resource_db::add_resource(&a, &user, "project", None)
        .await
        .unwrap();
    let hash = random_hex_32();
    let blob_a = blob_db::add_blob(&a, &hash, "stl", 12, None).await.unwrap();
    let model = model_db::add_model(&a, &user, "model", blob_a, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    resource_db::set_resource_on_group(&a, &user, Some(project.id), group.id, None)
        .await
        .unwrap();
    model_db::trash_models(&a, &user, &[model]).await.unwrap();
    let deleted: String = sqlx::query_scalar("SELECT model_deleted FROM models WHERE model_id=?")
        .bind(model)
        .fetch_one(&a)
        .await
        .unwrap();
    let capture = causal_adoption::observe_populated_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let bundle = causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture)
        .await
        .unwrap();
    let blob_b = blob_db::add_blob(&b, &hash, "stl", 12, None).await.unwrap();
    let peer = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let mappings = [ModelBlobBinding {
        entity: bundle.model_envelopes()[0].entity.clone(),
        local_blob_id: blob_b,
    }];
    let installed = causal_adoption::install_observed_populated_group(
        &b, &user, &bi, &peer, &bundle, &mappings,
    )
    .await
    .unwrap();
    let actual: String = sqlx::query_scalar("SELECT model_deleted FROM models WHERE model_id=?")
        .bind(installed.models[0].1)
        .fetch_one(&b)
        .await
        .unwrap();
    assert_eq!(actual, deleted);
    group_db::edit_group(&b, &user, installed.group_id, "renamed", None, None)
        .await
        .unwrap();
    let after_rename: String =
        sqlx::query_scalar("SELECT model_deleted FROM models WHERE model_id=?")
            .bind(installed.models[0].1)
            .fetch_one(&b)
            .await
            .unwrap();
    assert_eq!(
        after_rename, deleted,
        "group rename must not restore or re-trash a model"
    );
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(model_db::purge_trashed_models(&b, 0).await.unwrap(), 1);
    let row: (String, Option<i64>) =
        sqlx::query_as("SELECT group_name,group_resource_id FROM models_group WHERE group_id=?")
            .bind(installed.group_id)
            .fetch_one(&b)
            .await
            .unwrap();
    assert_eq!(row.0, "renamed");
    assert!(
        row.1.is_some(),
        "purging the last model must retain the project"
    );
    let state = db::causal_group_lifecycle::get_state(&b, &user, &bi, installed.group_id)
        .await
        .unwrap();
    assert!(matches!(
        state.name,
        db::causal_group_rename::NameState::Paused { .. }
    ));
    assert!(
        causal_adoption::install_observed_populated_group(
            &b, &user, &bi, &peer, &bundle, &mappings
        )
        .await
        .is_err(),
        "stale retry must not recreate a purged model"
    );
}

#[tokio::test]
async fn trashed_observation_rejects_restore_trash_aba_and_rolls_back_late_failure() {
    for scenario in ["aba", "failure"] {
        let (_dir, db) = common::test_db().await;
        let user = User::default();
        let identity = user_db::get_sync_identity(&db, user.id)
            .await
            .unwrap()
            .unwrap();
        let group = common::legacy_group(&db, &user, "group").await;
        let model = common::legacy_model(&db, &user, &random_hex_32()).await;
        group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
            .await
            .unwrap();
        model_db::trash_models(&db, &user, &[model]).await.unwrap();
        let capture = causal_adoption::observe_populated_group(&db, &user, &identity, group.id)
            .await
            .unwrap();
        if scenario == "aba" {
            model_db::restore_models(&db, &user, &[model])
                .await
                .unwrap();
            model_db::trash_models(&db, &user, &[model]).await.unwrap();
        } else {
            sqlx::query("CREATE TRIGGER reject_second_trash_binding BEFORE INSERT ON causal_genesis_bindings WHEN (SELECT COUNT(*) FROM causal_genesis_bindings)>=1 BEGIN SELECT RAISE(ABORT,'injected binding failure'); END").execute(&db).await.unwrap();
        }
        let before: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM entity_births),(SELECT COUNT(*) FROM causal_events),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
        assert!(
            causal_adoption::enroll_observed_populated_group(&db, &user, &identity, &capture)
                .await
                .is_err(),
            "{scenario} must reject atomically"
        );
        let after: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM entity_births),(SELECT COUNT(*) FROM causal_events),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
        assert_eq!(after, before);
        let births: (Option<String>,Option<String>) = sqlx::query_as("SELECT (SELECT group_birth_id FROM models_group WHERE group_id=?),(SELECT model_birth_id FROM models WHERE model_id=?)").bind(group.id).bind(model).fetch_one(&db).await.unwrap();
        assert_eq!(births, (None, None));
    }
}

#[tokio::test]
async fn forged_independent_move_of_observed_trashed_model_is_never_applied() {
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
    let blob = blob_db::add_blob(&a, &hash, "stl", 1, None).await.unwrap();
    let model = model_db::add_model(&a, &user, "model", blob, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    model_db::trash_models(&a, &user, &[model]).await.unwrap();
    let capture = causal_adoption::observe_populated_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let bundle = causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let blob_b = blob_db::add_blob(&b, &hash, "stl", 1, None).await.unwrap();
    let mappings = [ModelBlobBinding {
        entity: bundle.model_envelopes()[0].entity.clone(),
        local_blob_id: blob_b,
    }];
    let installed = causal_adoption::install_observed_populated_group(
        &b, &user, &bi, &peer, &bundle, &mappings,
    )
    .await
    .unwrap();
    let root = &bundle.model_envelopes()[0];
    let writer = random_hex_32();
    let dot = db::causal_envelope::Dot {
        writer_id: writer.clone(),
        counter: "1".into(),
    };
    let forged = db::causal_envelope::CausalEnvelope {
        version: 2,
        entity: root.entity.clone(),
        event: dot.clone(),
        parents: vec![root.event.clone()],
        dependencies: vec![],
        action: db::causal_envelope::Action::ModelMoved {
            from: Some(bundle.group_entity().clone()),
            to: None,
            target_lifecycle: None,
        },
    };
    let operation = random_hex_32();
    sqlx::query("INSERT INTO causal_events(user_id,writer_id,counter,envelope) VALUES (?,?,1,?)")
        .bind(user.id)
        .bind(&writer)
        .bind(db::causal_envelope::canonical_causal_envelope(&forged).unwrap())
        .execute(&a)
        .await
        .unwrap();
    sqlx::query("INSERT INTO group_lifecycle_operations(user_id,operation_id,focus_birth_id,event_count) VALUES (?,?,?,1)").bind(user.id).bind(&operation).bind(bundle.group_entity().birth_id.as_ref().unwrap()).execute(&a).await.unwrap();
    sqlx::query("INSERT INTO group_lifecycle_events(user_id,operation_id,position,writer_id,counter) VALUES (?,?,0,?,1)").bind(user.id).bind(&operation).bind(&writer).execute(&a).await.unwrap();
    let before: (Option<i64>,Option<String>,i64) = sqlx::query_as("SELECT model_group_id,model_deleted,(SELECT COUNT(*) FROM causal_events) FROM models WHERE model_id=?").bind(installed.models[0].1).fetch_one(&b).await.unwrap();
    // Origin export already refuses the forged move (first defence); if it ever
    // accepts, import must still reject it. Both outcomes are asserted.
    match db::causal_group_lifecycle::export_operation(&a, &user, &ai, &dot).await {
        Err(error) => assert!(
            matches!(error, db::DbError::Protocol(_)),
            "export must reject the forged move as a protocol error, got {error:?}"
        ),
        Ok(op) => assert!(
            db::causal_group_lifecycle::import_operation(&b, &user, &bi, &peer, &op)
                .await
                .is_err(),
            "forged independent move must not be admitted"
        ),
    }
    let after: (Option<i64>,Option<String>,i64) = sqlx::query_as("SELECT model_group_id,model_deleted,(SELECT COUNT(*) FROM causal_events) FROM models WHERE model_id=?").bind(installed.models[0].1).fetch_one(&b).await.unwrap();
    assert_eq!(after, before);
}
