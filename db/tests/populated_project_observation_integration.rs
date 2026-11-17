//! Project receipts preserve association and do not infer identity from globals.
mod common;
use db::{
    blob_db,
    causal_adoption::{self, ModelBlobBinding},
    group_db,
    model::user::User,
    model_db, random_hex_32, resource_db, sync_peer, user_db,
};

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn populated_project_observation_preserves_project_and_assignment() {
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
    sqlx::query("UPDATE resources SET resource_flags=17 WHERE resource_id=?")
        .bind(project.id)
        .execute(&a)
        .await
        .unwrap();
    let hash = random_hex_32();
    let blob = blob_db::add_blob(&a, &hash, "stl", 12, None).await.unwrap();
    let model = model_db::add_model(&a, &user, "model", blob, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    resource_db::set_resource_on_group(&a, &user, Some(project.id), group.id, None)
        .await
        .unwrap();
    let capture = causal_adoption::observe_populated_group(&a, &user, &ai, group.id).await;
    assert!(
        capture.is_ok(),
        "a populated group with an owned project must be observable: {capture:?}"
    );
    let bundle =
        causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture.unwrap())
            .await
            .unwrap();
    let peer = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let target_blob = blob_db::add_blob(&b, &hash, "stl", 12, None).await.unwrap();
    let mappings = [ModelBlobBinding {
        entity: bundle.model_envelopes()[0].entity.clone(),
        local_blob_id: target_blob,
    }];
    let installed = causal_adoption::install_observed_populated_group(
        &b, &user, &bi, &peer, &bundle, &mappings,
    )
    .await
    .unwrap();
    let source: (String,i64,String,String,String) = sqlx::query_as("SELECT resource_name,resource_flags,resource_created,resource_last_modified,resource_unique_global_id FROM resources WHERE resource_id=?").bind(project.id).fetch_one(&a).await.unwrap();
    let target: (String,i64,String,String,String) = sqlx::query_as("SELECT r.resource_name,r.resource_flags,r.resource_created,r.resource_last_modified,r.resource_unique_global_id FROM models_group g JOIN resources r ON r.resource_id=g.group_resource_id WHERE g.group_id=? AND g.group_user_id=? AND r.resource_user_id=?").bind(installed.group_id).bind(user.id).bind(user.id).fetch_one(&b).await.unwrap();
    assert_eq!(target, source);
    let second = common::legacy_group(&a, &user, "second").await;
    let hash2 = random_hex_32();
    let blob2 = blob_db::add_blob(&a, &hash2, "stl", 24, None)
        .await
        .unwrap();
    let model2 = model_db::add_model(&a, &user, "second model", blob2, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(second.id), vec![model2], None)
        .await
        .unwrap();
    resource_db::set_resource_on_group(&a, &user, Some(project.id), second.id, Some(&source.3))
        .await
        .unwrap();
    let capture2 = causal_adoption::observe_populated_group(&a, &user, &ai, second.id)
        .await
        .unwrap();
    let bundle2 = causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture2)
        .await
        .unwrap();
    let target_blob2 = blob_db::add_blob(&b, &hash2, "stl", 24, None)
        .await
        .unwrap();
    let second_blob = [ModelBlobBinding {
        entity: bundle2.model_envelopes()[0].entity.clone(),
        local_blob_id: target_blob2,
    }];
    let installed2 = causal_adoption::install_observed_populated_group(
        &b,
        &user,
        &bi,
        &peer,
        &bundle2,
        &second_blob,
    )
    .await
    .unwrap();
    let project_ids: (Option<i64>,Option<i64>) = sqlx::query_as("SELECT (SELECT group_resource_id FROM models_group WHERE group_id=?),(SELECT group_resource_id FROM models_group WHERE group_id=?)").bind(installed.group_id).bind(installed2.group_id).fetch_one(&b).await.unwrap();
    assert!(project_ids.0.is_some());
    assert_eq!(
        project_ids.0, project_ids.1,
        "verified identical source projects must be reused"
    );
    sqlx::query("UPDATE resources SET resource_flags=16 WHERE resource_id=?")
        .bind(project_ids.0.unwrap())
        .execute(&b)
        .await
        .unwrap();
    sqlx::query("UPDATE resources SET resource_flags=17 WHERE resource_id=?")
        .bind(project_ids.0.unwrap())
        .execute(&b)
        .await
        .unwrap();
    for group_id in [installed.group_id, installed2.group_id] {
        let state = db::causal_group_lifecycle::get_state(&b, &user, &bi, group_id)
            .await
            .unwrap();
        assert!(
            matches!(
                state.name,
                db::causal_group_rename::NameState::Paused { .. }
            ),
            "project ABA must pause each associated scope"
        );
    }
    let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models WHERE model_group_id=?")
        .bind(installed.group_id)
        .fetch_one(&b)
        .await
        .unwrap();
    assert_eq!(members, 1);
    model_db::trash_models(&b, &user, &[installed.models[0].1])
        .await
        .unwrap();
    let retained: Option<i64> =
        sqlx::query_scalar("SELECT group_resource_id FROM models_group WHERE group_id=?")
            .bind(installed.group_id)
            .fetch_one(&b)
            .await
            .unwrap();
    assert!(
        retained.is_some(),
        "trashing all members must preserve the project"
    );
    let state = db::causal_group_lifecycle::get_state(&b, &user, &bi, installed.group_id)
        .await
        .unwrap();
    assert!(matches!(
        state.name,
        db::causal_group_rename::NameState::Paused { .. }
    ));
}

#[tokio::test]
async fn emptied_project_group_is_observable_and_installs_without_models() {
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
    let group = common::legacy_group(&a, &user, "emptied").await;
    let project = resource_db::add_resource(&a, &user, "project", None)
        .await
        .unwrap();
    let blob = blob_db::add_blob(&a, &random_hex_32(), "stl", 12, None)
        .await
        .unwrap();
    let model = model_db::add_model(&a, &user, "model", blob, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    resource_db::set_resource_on_group(&a, &user, Some(project.id), group.id, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, None, vec![model], None)
        .await
        .unwrap();
    let capture = causal_adoption::observe_populated_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let bundle = causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture)
        .await
        .unwrap();
    assert_eq!(bundle.model_envelopes().len(), 0);
    let peer = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let installed =
        causal_adoption::install_observed_populated_group(&b, &user, &bi, &peer, &bundle, &[])
            .await
            .unwrap();
    assert_eq!(installed.models.len(), 0);
    let name: String = sqlx::query_scalar("SELECT r.resource_name FROM models_group g JOIN resources r ON r.resource_id=g.group_resource_id WHERE g.group_id=? AND g.group_user_id=?").bind(installed.group_id).bind(user.id).fetch_one(&b).await.unwrap();
    assert_eq!(name, "project");
}

#[tokio::test]
async fn project_or_association_aba_invalidates_whole_capture() {
    for change in ["project", "association"] {
        let (_dir, db) = common::test_db().await;
        let user = User::default();
        let identity = user_db::get_sync_identity(&db, user.id)
            .await
            .unwrap()
            .unwrap();
        let group = common::legacy_group(&db, &user, "group").await;
        let project = resource_db::add_resource(&db, &user, "project", None)
            .await
            .unwrap();
        let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 1, None)
            .await
            .unwrap();
        let model = model_db::add_model(&db, &user, "model", blob, None, None)
            .await
            .unwrap();
        group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
            .await
            .unwrap();
        resource_db::set_resource_on_group(&db, &user, Some(project.id), group.id, None)
            .await
            .unwrap();
        let capture = causal_adoption::observe_populated_group(&db, &user, &identity, group.id)
            .await
            .unwrap();
        match change {
            "project" => {
                sqlx::query("UPDATE resources SET resource_flags=17 WHERE resource_id=?")
                    .bind(project.id)
                    .execute(&db)
                    .await
                    .unwrap();
                sqlx::query("UPDATE resources SET resource_flags=0 WHERE resource_id=?")
                    .bind(project.id)
                    .execute(&db)
                    .await
                    .unwrap();
            }
            "association" => {
                resource_db::set_resource_on_group(&db, &user, None, group.id, None)
                    .await
                    .unwrap();
                resource_db::set_resource_on_group(&db, &user, Some(project.id), group.id, None)
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
            "{change} ABA must invalidate the complete capture"
        );
        let after: (i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
        assert_eq!(after, before);
    }
}

#[tokio::test]
async fn unrelated_project_collision_and_receipt_failure_leave_target_unchanged() {
    let (_ad, a) = common::test_db().await;
    let (_bd, b) = common::test_db().await;
    let (_cd, c) = common::test_db().await;
    let user = User::default();
    let ai = user_db::get_sync_identity(&a, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = common::legacy_group(&a, &user, "group").await;
    let project = resource_db::add_resource(&a, &user, "source project", None)
        .await
        .unwrap();
    let hash = random_hex_32();
    let blob = blob_db::add_blob(&a, &hash, "stl", 12, None).await.unwrap();
    let model = model_db::add_model(&a, &user, "model", blob, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    resource_db::set_resource_on_group(&a, &user, Some(project.id), group.id, None)
        .await
        .unwrap();
    let capture = causal_adoption::observe_populated_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let bundle = causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture)
        .await
        .unwrap();
    for (db, collision) in [(&b, true), (&c, false)] {
        let identity = user_db::get_sync_identity(db, user.id)
            .await
            .unwrap()
            .unwrap();
        let peer = sync_peer::bind_peer(db, &user, &identity, &ai)
            .await
            .unwrap();
        let blob = blob_db::add_blob(db, &hash, "stl", 12, None).await.unwrap();
        let mapping = [ModelBlobBinding {
            entity: bundle.model_envelopes()[0].entity.clone(),
            local_blob_id: blob,
        }];
        if collision {
            let unrelated = resource_db::add_resource(db, &user, "unrelated", None)
                .await
                .unwrap();
            sqlx::query("UPDATE resources SET resource_unique_global_id=? WHERE resource_id=?")
                .bind(&project.unique_global_id)
                .bind(unrelated.id)
                .execute(db)
                .await
                .unwrap();
        } else {
            sqlx::query("CREATE TRIGGER reject_project_receipt BEFORE INSERT ON observed_project_bindings BEGIN SELECT RAISE(ABORT,'injected project receipt failure'); END").execute(db).await.unwrap();
        }
        let before: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM resources),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM causal_genesis_bindings)").fetch_one(db).await.unwrap();
        assert!(
            causal_adoption::install_observed_populated_group(
                db, &user, &identity, &peer, &bundle, &mapping
            )
            .await
            .is_err()
        );
        let after: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM resources),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM causal_genesis_bindings)").fetch_one(db).await.unwrap();
        assert_eq!(
            after, before,
            "collision or late receipt failure must not partially install"
        );
    }
}

#[tokio::test]
async fn foreign_project_relationship_cannot_be_observed() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let other_id = user_db::add_user(&db, "other", "project-other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let foreign = resource_db::add_resource(&db, &other, "foreign", None)
        .await
        .unwrap();
    let group = common::legacy_group(&db, &user, "group").await;
    let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "model", blob, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    sqlx::query("UPDATE models_group SET group_resource_id=? WHERE group_id=?")
        .bind(foreign.id)
        .bind(group.id)
        .execute(&db)
        .await
        .unwrap();
    assert!(
        causal_adoption::observe_populated_group(&db, &user, &identity, group.id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn project_snapshots_count_toward_combined_byte_limit() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let timestamp = "2020-01-01T00:00:00Z";
    let project = resource_db::add_resource(&db, &user, &"p".repeat(60_000), Some(timestamp))
        .await
        .unwrap();
    let limit = i64::try_from(db::causal_store::MAX_OWNER_BYTES).unwrap();
    let mut previous = 0_i64;
    let mut last = 0_i64;
    for _ in 0..400 {
        let total: i64 = sqlx::query_scalar("SELECT COALESCE((SELECT SUM(length(CAST(e.envelope AS BLOB))+length(CAST(b.materialization AS BLOB))+length(CAST(b.local_guard AS BLOB))+COALESCE(length(CAST(b.observation_evidence AS BLOB)),0)) FROM causal_genesis_bindings b JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.capability='group_lifecycle_v1'),0)+COALESCE((SELECT SUM(length(CAST(manifest AS BLOB))+COALESCE(length(CAST(label_snapshot AS BLOB)),0)+COALESCE(length(CAST(project_snapshot AS BLOB)),0)) FROM causal_observation_batches WHERE user_id=?),0)").bind(user.id).bind(user.id).fetch_one(&db).await.unwrap();
        let group = common::legacy_group(&db, &user, "group").await;
        let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 1, None)
            .await
            .unwrap();
        let model = model_db::add_model(&db, &user, "model", blob, None, None)
            .await
            .unwrap();
        group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
            .await
            .unwrap();
        resource_db::set_resource_on_group(&db, &user, Some(project.id), group.id, Some(timestamp))
            .await
            .unwrap();
        let capture = causal_adoption::observe_populated_group(&db, &user, &identity, group.id)
            .await
            .unwrap();
        let is_final = last > previous && total + (last - previous) > limit + 1024;
        let before: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT COUNT(*) FROM causal_observation_batches),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
        let result =
            causal_adoption::enroll_observed_populated_group(&db, &user, &identity, &capture).await;
        if is_final {
            assert!(
                result.is_err(),
                "project snapshot bytes must count toward the aggregate limit"
            );
            let after: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT COUNT(*) FROM causal_observation_batches),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
            assert_eq!(after, before);
            return;
        }
        result.unwrap();
        previous = total;
        last = sqlx::query_scalar("SELECT COALESCE((SELECT SUM(length(CAST(e.envelope AS BLOB))+length(CAST(b.materialization AS BLOB))+length(CAST(b.local_guard AS BLOB))+COALESCE(length(CAST(b.observation_evidence AS BLOB)),0)) FROM causal_genesis_bindings b JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.capability='group_lifecycle_v1'),0)+COALESCE((SELECT SUM(length(CAST(manifest AS BLOB))+COALESCE(length(CAST(label_snapshot AS BLOB)),0)+COALESCE(length(CAST(project_snapshot AS BLOB)),0)) FROM causal_observation_batches WHERE user_id=?),0)").bind(user.id).bind(user.id).fetch_one(&db).await.unwrap();
    }
    panic!("production byte limit was not reached");
}

#[tokio::test]
async fn late_rename_keeps_purged_group_and_original_project_incarnation() {
    use db::causal_group_lifecycle::{self, GroupLifecycle, OperationImportResult};
    use db::causal_group_rename::NameState;
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
    let incarnation: String =
        sqlx::query_scalar("SELECT resource_incarnation FROM resources WHERE resource_id=?")
            .bind(project.id)
            .fetch_one(&a)
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
    group_db::delete_group(&a, &user, group.id).await.unwrap();
    group_db::edit_group(&b, &user, installed.group_id, "offline keep", None, None)
        .await
        .unwrap();
    let state = causal_group_lifecycle::get_state(&b, &user, &bi, installed.group_id)
        .await
        .unwrap();
    let dot = match state.name {
        NameState::Ready { token, .. } => token.frontier[0].clone(),
        other => panic!("expected ready: {other:?}"),
    };
    let operation = causal_group_lifecycle::export_operation(&b, &user, &bi, &dot)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(group_db::purge_trashed_groups(&a, 0).await.unwrap(), 1);
    let peer_a = sync_peer::bind_peer(&a, &user, &ai, &bi).await.unwrap();
    let imported =
        causal_group_lifecycle::import_operation(&a, &user, &ai, &peer_a, &operation).await;
    assert!(
        imported.is_ok(),
        "late keep must recover the original project: {imported:?}"
    );
    let imported = imported.unwrap();
    assert!(
        matches!(imported,OperationImportResult::Applied(ref state) if matches!(state.lifecycle,GroupLifecycle::Active))
    );
    let row: (i64,String,Option<i64>,String) = sqlx::query_as("SELECT g.group_id,g.group_name,g.group_resource_id,r.resource_incarnation FROM models_group g JOIN resources r ON r.resource_id=g.group_resource_id WHERE g.group_user_id=? AND g.group_unique_global_id=?").bind(user.id).bind(&group.unique_global_id).fetch_one(&a).await.unwrap();
    assert_ne!(row.0, group.id);
    assert_eq!(
        (row.1, row.2, row.3),
        ("offline keep".into(), Some(project.id), incarnation)
    );
    let assignment: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id=?")
            .bind(model)
            .fetch_one(&a)
            .await
            .unwrap();
    assert_eq!(assignment, Some(row.0));
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn replaced_target_project_pauses_populated_and_empty_groups() {
    use db::causal_group_rename::NameState;
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
    let project = resource_db::add_resource(&a, &user, "project", None)
        .await
        .unwrap();
    let group = common::legacy_group(&a, &user, "populated").await;
    let empty = common::legacy_group(&a, &user, "empty").await;
    let hash = random_hex_32();
    let blob_a = blob_db::add_blob(&a, &hash, "stl", 12, None).await.unwrap();
    let model = model_db::add_model(&a, &user, "model", blob_a, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(empty.id), vec![model], None)
        .await
        .unwrap();
    resource_db::set_resource_on_group(
        &a,
        &user,
        Some(project.id),
        empty.id,
        Some(&project.last_modified),
    )
    .await
    .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    resource_db::set_resource_on_group(
        &a,
        &user,
        Some(project.id),
        group.id,
        Some(&project.last_modified),
    )
    .await
    .unwrap();
    let group_capture = causal_adoption::observe_populated_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let group_bundle =
        causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &group_capture)
            .await
            .unwrap();
    let empty_capture = causal_adoption::observe_populated_group(&a, &user, &ai, empty.id)
        .await
        .unwrap();
    let empty_bundle =
        causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &empty_capture)
            .await
            .unwrap();
    let peer = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let blob_b = blob_db::add_blob(&b, &hash, "stl", 12, None).await.unwrap();
    let mappings = [ModelBlobBinding {
        entity: group_bundle.model_envelopes()[0].entity.clone(),
        local_blob_id: blob_b,
    }];
    let installed = causal_adoption::install_observed_populated_group(
        &b,
        &user,
        &bi,
        &peer,
        &group_bundle,
        &mappings,
    )
    .await
    .unwrap();
    let installed_empty = causal_adoption::install_observed_populated_group(
        &b,
        &user,
        &bi,
        &peer,
        &empty_bundle,
        &[],
    )
    .await
    .unwrap();
    let project_b: i64 =
        sqlx::query_scalar("SELECT group_resource_id FROM models_group WHERE group_id=?")
            .bind(installed.group_id)
            .fetch_one(&b)
            .await
            .unwrap();
    let incarnation: String =
        sqlx::query_scalar("SELECT resource_incarnation FROM resources WHERE resource_id=?")
            .bind(project_b)
            .fetch_one(&b)
            .await
            .unwrap();
    resource_db::delete_resource(&b, &user, project_b)
        .await
        .unwrap();
    let replacement = resource_db::add_resource(&b, &user, "project", None)
        .await
        .unwrap();
    sqlx::query("UPDATE resources SET resource_unique_global_id=? WHERE resource_id=?")
        .bind(&project.unique_global_id)
        .bind(replacement.id)
        .execute(&b)
        .await
        .unwrap();
    let new_incarnation: String =
        sqlx::query_scalar("SELECT resource_incarnation FROM resources WHERE resource_id=?")
            .bind(replacement.id)
            .fetch_one(&b)
            .await
            .unwrap();
    assert_ne!(incarnation, new_incarnation);
    for id in [installed.group_id, installed_empty.group_id] {
        let state = db::causal_group_lifecycle::get_state(&b, &user, &bi, id)
            .await
            .unwrap();
        assert!(
            matches!(state.name, NameState::Paused { .. }),
            "replacement project must pause each old binding"
        );
        let association: Option<i64> =
            sqlx::query_scalar("SELECT group_resource_id FROM models_group WHERE group_id=?")
                .bind(id)
                .fetch_one(&b)
                .await
                .unwrap();
        assert_ne!(association, Some(replacement.id));
    }
    let retry = causal_adoption::install_observed_populated_group(
        &b,
        &user,
        &bi,
        &peer,
        &group_bundle,
        &mappings,
    )
    .await;
    assert!(
        retry.is_err(),
        "replacement must not satisfy the old source-project receipt"
    );
    let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models WHERE model_group_id=?")
        .bind(installed.group_id)
        .fetch_one(&b)
        .await
        .unwrap();
    assert_eq!(members, 1);
}
