//! Label snapshots are scoped observation evidence, not inferred shared identity.
mod common;
use db::{
    blob_db,
    causal_adoption::{self, ModelBlobBinding},
    group_db, label_db,
    model::user::User,
    model_db, random_hex_32, sync_peer, user_db,
};

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn observation_installs_shared_labels_and_complete_hierarchy() {
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
    let model1 = model_db::add_model(&a, &user, "one", blob1, None, None)
        .await
        .unwrap();
    let model2 = model_db::add_model(&a, &user, "two", blob2, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(group.id), vec![model1, model2], None)
        .await
        .unwrap();
    let parent = label_db::add_label(&a, &user, "parent", 11, None)
        .await
        .unwrap();
    let child = label_db::add_label(&a, &user, "child", 22, None)
        .await
        .unwrap();
    let sibling = label_db::add_label(&a, &user, "sibling", 33, None)
        .await
        .unwrap();
    label_db::set_childs_on_label(&a, &user, parent.id, &[child.id, sibling.id])
        .await
        .unwrap();
    label_db::add_labels_on_models(&a, &user, &[child.id], &[model1, model2], None)
        .await
        .unwrap();
    let capture = causal_adoption::observe_populated_group(&a, &user, &ai, group.id).await;
    assert!(
        capture.is_ok(),
        "complete owned shared labels and parents should be observable: {capture:?}"
    );
    let bundle =
        causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture.unwrap())
            .await
            .unwrap();
    let blob_b1 = blob_db::add_blob(&b, &hash1, "stl", 12, None)
        .await
        .unwrap();
    let blob_b2 = blob_db::add_blob(&b, &hash2, "obj", 34, None)
        .await
        .unwrap();
    let model1_global: String =
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
            local_blob_id: if event.entity.global_id == model1_global {
                blob_b1
            } else {
                blob_b2
            },
        })
        .collect();
    let peer = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let installed = causal_adoption::install_observed_populated_group(
        &b, &user, &bi, &peer, &bundle, &mappings,
    )
    .await
    .unwrap();
    let labels: Vec<(String, i64)> = sqlx::query_as(
        "SELECT label_name,label_color FROM labels WHERE label_user_id=? ORDER BY label_name",
    )
    .bind(user.id)
    .fetch_all(&b)
    .await
    .unwrap();
    assert_eq!(
        labels,
        vec![
            ("child".into(), 22),
            ("parent".into(), 11),
            ("sibling".into(), 33)
        ]
    );
    let edges: Vec<(String,String)> = sqlx::query_as("SELECT parent.label_name,child.label_name FROM labels_labels e JOIN labels parent ON parent.label_id=e.parent_label_id JOIN labels child ON child.label_id=e.child_label_id WHERE parent.label_user_id=? ORDER BY child.label_name").bind(user.id).fetch_all(&b).await.unwrap();
    assert_eq!(
        edges,
        vec![
            ("parent".into(), "child".into()),
            ("parent".into(), "sibling".into())
        ]
    );
    let assignments: Vec<(String,String)> = sqlx::query_as("SELECT m.model_name,l.label_name FROM models m JOIN models_labels ml ON ml.model_id=m.model_id JOIN labels l ON l.label_id=ml.label_id WHERE m.model_group_id=? ORDER BY m.model_name").bind(installed.group_id).fetch_all(&b).await.unwrap();
    assert_eq!(
        assignments,
        vec![
            ("one".into(), "child".into()),
            ("two".into(), "child".into())
        ]
    );
    let second = common::legacy_group(&a, &user, "second").await;
    let hash3 = random_hex_32();
    let blob3 = blob_db::add_blob(&a, &hash3, "stl", 56, None)
        .await
        .unwrap();
    let model3 = model_db::add_model(&a, &user, "three", blob3, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(second.id), vec![model3], None)
        .await
        .unwrap();
    let modified: String =
        sqlx::query_scalar("SELECT label_last_modified FROM labels WHERE label_id=?")
            .bind(child.id)
            .fetch_one(&a)
            .await
            .unwrap();
    label_db::add_labels_on_models(&a, &user, &[child.id], &[model3], Some(&modified))
        .await
        .unwrap();
    let capture2 = causal_adoption::observe_populated_group(&a, &user, &ai, second.id)
        .await
        .unwrap();
    let bundle2 = causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture2)
        .await
        .unwrap();
    let blob_b3 = blob_db::add_blob(&b, &hash3, "stl", 56, None)
        .await
        .unwrap();
    let second_blobs = [ModelBlobBinding {
        entity: bundle2.model_envelopes()[0].entity.clone(),
        local_blob_id: blob_b3,
    }];
    causal_adoption::install_observed_populated_group(
        &b,
        &user,
        &bi,
        &peer,
        &bundle2,
        &second_blobs,
    )
    .await
    .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM labels WHERE label_user_id=?")
        .bind(user.id)
        .fetch_one(&b)
        .await
        .unwrap();
    assert_eq!(count, 3, "identical verified source labels must be reused");
    let third = common::legacy_group(&a, &user, "third").await;
    let hash4 = random_hex_32();
    let blob4 = blob_db::add_blob(&a, &hash4, "stl", 78, None)
        .await
        .unwrap();
    let model4 = model_db::add_model(&a, &user, "four", blob4, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(third.id), vec![model4], None)
        .await
        .unwrap();
    label_db::add_labels_on_models(&a, &user, &[child.id], &[model4], Some(&modified))
        .await
        .unwrap();
    let capture3 = causal_adoption::observe_populated_group(&a, &user, &ai, third.id)
        .await
        .unwrap();
    let bundle3 = causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture3)
        .await
        .unwrap();
    let blob_b4 = blob_db::add_blob(&b, &hash4, "stl", 78, None)
        .await
        .unwrap();
    let third_blobs = [ModelBlobBinding {
        entity: bundle3.model_envelopes()[0].entity.clone(),
        local_blob_id: blob_b4,
    }];
    let target: (i64, String) = sqlx::query_as("SELECT label_id,label_last_modified FROM labels WHERE label_user_id=? AND label_name='child'").bind(user.id).fetch_one(&b).await.unwrap();
    label_db::edit_label(&b, &user, target.0, "child", 23, Some(&target.1), None)
        .await
        .unwrap();
    label_db::edit_label(&b, &user, target.0, "child", 22, Some(&target.1), None)
        .await
        .unwrap();
    let state = db::causal_group_lifecycle::get_state(&b, &user, &bi, installed.group_id)
        .await
        .unwrap();
    assert!(
        matches!(
            state.name,
            db::causal_group_rename::NameState::Paused { .. }
        ),
        "label metadata ABA must pause the observed scope"
    );
    group_db::edit_group(&b, &user, installed.group_id, "local edit", None, None)
        .await
        .unwrap();
    let state = db::causal_group_lifecycle::get_state(&b, &user, &bi, installed.group_id)
        .await
        .unwrap();
    assert!(matches!(
        state.name,
        db::causal_group_rename::NameState::Paused { .. }
    ));
    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM models WHERE model_group_id=?")
            .bind(installed.group_id)
            .fetch_one(&b)
            .await
            .unwrap();
    assert_eq!(
        member_count, 2,
        "pausing must preserve the existing assignments"
    );
    let before: (i64, i64) =
        sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM models)")
            .fetch_one(&b)
            .await
            .unwrap();
    assert!(
        causal_adoption::install_observed_populated_group(
            &b,
            &user,
            &bi,
            &peer,
            &bundle3,
            &third_blobs
        )
        .await
        .is_err(),
        "target label ABA must invalidate the reuse receipt"
    );
    let after: (i64, i64) =
        sqlx::query_as("SELECT (SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM models)")
            .fetch_one(&b)
            .await
            .unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn observed_label_edits_and_hierarchy_aba_invalidate_capture() {
    for change in ["metadata", "hierarchy", "assignment"] {
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
        let model = model_db::add_model(&db, &user, "model", blob, None, None)
            .await
            .unwrap();
        group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
            .await
            .unwrap();
        let parent = label_db::add_label(&db, &user, "parent", 11, None)
            .await
            .unwrap();
        let child = label_db::add_label(&db, &user, "child", 22, None)
            .await
            .unwrap();
        label_db::set_childs_on_label(&db, &user, parent.id, &[child.id])
            .await
            .unwrap();
        label_db::add_labels_on_models(&db, &user, &[child.id], &[model], None)
            .await
            .unwrap();
        let capture = causal_adoption::observe_populated_group(&db, &user, &identity, group.id)
            .await
            .unwrap();
        match change {
            "metadata" => {
                label_db::edit_label(
                    &db,
                    &user,
                    child.id,
                    "changed",
                    22,
                    Some(&child.last_modified),
                    None,
                )
                .await
                .unwrap();
                label_db::edit_label(
                    &db,
                    &user,
                    child.id,
                    "child",
                    22,
                    Some(&child.last_modified),
                    None,
                )
                .await
                .unwrap();
            }
            "hierarchy" => {
                label_db::set_childs_on_label(&db, &user, parent.id, &[])
                    .await
                    .unwrap();
                label_db::set_childs_on_label(&db, &user, parent.id, &[child.id])
                    .await
                    .unwrap();
            }
            "assignment" => {
                label_db::remove_labels_from_models(&db, &user, &[child.id], &[model], None)
                    .await
                    .unwrap();
                label_db::add_labels_on_models(&db, &user, &[child.id], &[model], None)
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
            "{change} ABA must invalidate the capture"
        );
        let after: (i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
        assert_eq!(after, before);
    }
}

#[tokio::test]
async fn unrelated_same_global_label_is_not_adopted_and_install_failure_rolls_back() {
    let (_ad, a) = common::test_db().await;
    let (_bd, b) = common::test_db().await;
    let (_cd, c) = common::test_db().await;
    let user = User::default();
    let ai = user_db::get_sync_identity(&a, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = common::legacy_group(&a, &user, "group").await;
    let hash = random_hex_32();
    let blob = blob_db::add_blob(&a, &hash, "stl", 12, None).await.unwrap();
    let model = model_db::add_model(&a, &user, "model", blob, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&a, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    let label = label_db::add_label(&a, &user, "source", 7, None)
        .await
        .unwrap();
    label_db::add_labels_on_models(&a, &user, &[label.id], &[model], None)
        .await
        .unwrap();
    let capture = causal_adoption::observe_populated_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let bundle = causal_adoption::enroll_observed_populated_group(&a, &user, &ai, &capture)
        .await
        .unwrap();
    let bi = user_db::get_sync_identity(&b, user.id)
        .await
        .unwrap()
        .unwrap();
    let peer_b = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let blob_b = blob_db::add_blob(&b, &hash, "stl", 12, None).await.unwrap();
    let unrelated = label_db::add_label(&b, &user, "unrelated", 99, None)
        .await
        .unwrap();
    label_db::edit_label(
        &b,
        &user,
        unrelated.id,
        "unrelated",
        99,
        None,
        Some(&label.unique_global_id),
    )
    .await
    .unwrap();
    let bindings_b = [ModelBlobBinding {
        entity: bundle.model_envelopes()[0].entity.clone(),
        local_blob_id: blob_b,
    }];
    assert!(
        causal_adoption::install_observed_populated_group(
            &b,
            &user,
            &bi,
            &peer_b,
            &bundle,
            &bindings_b
        )
        .await
        .is_err()
    );
    let remaining: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM labels),(SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group)").fetch_one(&b).await.unwrap();
    assert_eq!(remaining, (1, 0, 0));
    let ci = user_db::get_sync_identity(&c, user.id)
        .await
        .unwrap()
        .unwrap();
    let peer_c = sync_peer::bind_peer(&c, &user, &ci, &ai).await.unwrap();
    let blob_c = blob_db::add_blob(&c, &hash, "stl", 12, None).await.unwrap();
    let bindings_c = [ModelBlobBinding {
        entity: bundle.model_envelopes()[0].entity.clone(),
        local_blob_id: blob_c,
    }];
    sqlx::query("CREATE TRIGGER reject_observed_label BEFORE INSERT ON observed_label_bindings BEGIN SELECT RAISE(ABORT,'injected label receipt failure'); END").execute(&c).await.unwrap();
    assert!(
        causal_adoption::install_observed_populated_group(
            &c,
            &user,
            &ci,
            &peer_c,
            &bundle,
            &bindings_c
        )
        .await
        .is_err()
    );
    let remaining: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM labels),(SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM causal_genesis_bindings)").fetch_one(&c).await.unwrap();
    assert_eq!(remaining, (0, 0, 0, 0));
}

#[tokio::test]
async fn observation_rejects_foreign_hierarchy_and_ambiguous_label_identity() {
    for invalid in ["foreign", "duplicate"] {
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
        let model = model_db::add_model(&db, &user, "model", blob, None, None)
            .await
            .unwrap();
        group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
            .await
            .unwrap();
        let label = label_db::add_label(&db, &user, "source", 7, None)
            .await
            .unwrap();
        label_db::add_labels_on_models(&db, &user, &[label.id], &[model], None)
            .await
            .unwrap();
        if invalid == "foreign" {
            let other_id =
                user_db::add_user(&db, "other", "other-label@example.test", &random_hex_32())
                    .await
                    .unwrap();
            let other = user_db::get_user_by_id(&db, other_id)
                .await
                .unwrap()
                .unwrap();
            let foreign = label_db::add_label(&db, &other, "foreign", 8, None)
                .await
                .unwrap();
            sqlx::query("INSERT INTO labels_labels(parent_label_id,child_label_id) VALUES (?,?)")
                .bind(label.id)
                .bind(foreign.id)
                .execute(&db)
                .await
                .unwrap();
        } else {
            let duplicate = label_db::add_label(&db, &user, "duplicate", 8, None)
                .await
                .unwrap();
            label_db::edit_label(
                &db,
                &user,
                duplicate.id,
                "duplicate",
                8,
                None,
                Some(&label.unique_global_id),
            )
            .await
            .unwrap();
        }
        assert!(
            causal_adoption::observe_populated_group(&db, &user, &identity, group.id)
                .await
                .is_err(),
            "{invalid} label cannot be silently included or resolved"
        );
    }
}

#[tokio::test]
async fn label_snapshots_count_toward_combined_byte_limit() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let timestamp = "2020-01-01T00:00:00Z";
    let label = label_db::add_label(&db, &user, &"l".repeat(60_000), 7, Some(timestamp))
        .await
        .unwrap();
    let limit = i64::try_from(db::causal_store::MAX_OWNER_BYTES).unwrap();
    let mut previous = 0_i64;
    let mut last = 0_i64;
    for _ in 0..400 {
        let total: i64 = sqlx::query_scalar("SELECT COALESCE((SELECT SUM(length(CAST(e.envelope AS BLOB))+length(CAST(b.materialization AS BLOB))+length(CAST(b.local_guard AS BLOB))+COALESCE(length(CAST(b.observation_evidence AS BLOB)),0)) FROM causal_genesis_bindings b JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.capability='group_lifecycle_v1'),0)+COALESCE((SELECT SUM(length(CAST(manifest AS BLOB))+COALESCE(length(CAST(label_snapshot AS BLOB)),0)) FROM causal_observation_batches WHERE user_id=?),0)").bind(user.id).bind(user.id).fetch_one(&db).await.unwrap();
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
        label_db::add_labels_on_models(&db, &user, &[label.id], &[model], Some(timestamp))
            .await
            .unwrap();
        let capture = causal_adoption::observe_populated_group(&db, &user, &identity, group.id)
            .await
            .unwrap();
        let is_final = last > previous && total + (last - previous) > limit + 1024;
        let before: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT COUNT(*) FROM causal_observation_batches),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
        let result =
            causal_adoption::enroll_observed_populated_group(&db, &user, &identity, &capture).await;
        if is_final {
            assert!(
                result.is_err(),
                "label snapshot bytes must count toward the aggregate limit"
            );
            let after: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT COUNT(*) FROM causal_observation_batches),(SELECT counter FROM group_causal_replica WHERE singleton=1)").fetch_one(&db).await.unwrap();
            assert_eq!(after, before, "capacity rejection must be atomic");
            return;
        }
        result.unwrap();
        previous = total;
        last = sqlx::query_scalar("SELECT COALESCE((SELECT SUM(length(CAST(e.envelope AS BLOB))+length(CAST(b.materialization AS BLOB))+length(CAST(b.local_guard AS BLOB))+COALESCE(length(CAST(b.observation_evidence AS BLOB)),0)) FROM causal_genesis_bindings b JOIN causal_events e USING(user_id,writer_id,counter) WHERE b.user_id=? AND b.capability='group_lifecycle_v1'),0)+COALESCE((SELECT SUM(length(CAST(manifest AS BLOB))+COALESCE(length(CAST(label_snapshot AS BLOB)),0)) FROM causal_observation_batches WHERE user_id=?),0)").bind(user.id).bind(user.id).fetch_one(&db).await.unwrap();
    }
    panic!("production byte limit was not reached");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn target_label_changes_pause_scope_without_losing_members() {
    use db::causal_group_rename::NameState;
    for change in ["hierarchy", "assignment", "replacement"] {
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
        let model = model_db::add_model(&a, &user, "model", blob_a, None, None)
            .await
            .unwrap();
        group_db::set_group_id_on_models(&a, &user, Some(group.id), vec![model], None)
            .await
            .unwrap();
        let parent = label_db::add_label(&a, &user, "parent", 7, None)
            .await
            .unwrap();
        let child = label_db::add_label(&a, &user, "child", 8, None)
            .await
            .unwrap();
        label_db::set_childs_on_label(&a, &user, parent.id, &[child.id])
            .await
            .unwrap();
        label_db::add_labels_on_models(&a, &user, &[child.id], &[model], None)
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
        let parent_b: i64 = sqlx::query_scalar(
            "SELECT label_id FROM labels WHERE label_user_id=? AND label_name='parent'",
        )
        .bind(user.id)
        .fetch_one(&b)
        .await
        .unwrap();
        let child_b: i64 = sqlx::query_scalar(
            "SELECT label_id FROM labels WHERE label_user_id=? AND label_name='child'",
        )
        .bind(user.id)
        .fetch_one(&b)
        .await
        .unwrap();
        let target_model = installed.models[0].1;
        match change {
            "hierarchy" => {
                label_db::set_childs_on_label(&b, &user, parent_b, &[])
                    .await
                    .unwrap();
                label_db::set_childs_on_label(&b, &user, parent_b, &[child_b])
                    .await
                    .unwrap();
            }
            "assignment" => {
                label_db::remove_labels_from_models(&b, &user, &[child_b], &[target_model], None)
                    .await
                    .unwrap();
                label_db::add_labels_on_models(&b, &user, &[child_b], &[target_model], None)
                    .await
                    .unwrap();
            }
            "replacement" => {
                let original_incarnation: String =
                    sqlx::query_scalar("SELECT label_incarnation FROM labels WHERE label_id=?")
                        .bind(child_b)
                        .fetch_one(&b)
                        .await
                        .unwrap();
                label_db::delete_label(&b, &user, child_b).await.unwrap();
                let replacement = label_db::add_label(&b, &user, "child", 8, None)
                    .await
                    .unwrap();
                let replacement_incarnation: String =
                    sqlx::query_scalar("SELECT label_incarnation FROM labels WHERE label_id=?")
                        .bind(replacement.id)
                        .fetch_one(&b)
                        .await
                        .unwrap();
                assert_ne!(replacement_incarnation, original_incarnation);
                label_db::edit_label(
                    &b,
                    &user,
                    replacement.id,
                    "child",
                    8,
                    None,
                    Some(&child.unique_global_id),
                )
                .await
                .unwrap();
                label_db::set_childs_on_label(&b, &user, parent_b, &[replacement.id])
                    .await
                    .unwrap();
                label_db::add_labels_on_models(&b, &user, &[replacement.id], &[target_model], None)
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let state = db::causal_group_lifecycle::get_state(&b, &user, &bi, installed.group_id)
            .await
            .unwrap();
        assert!(
            matches!(state.name, NameState::Paused { .. }),
            "{change} must pause observed scope"
        );
        let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models WHERE model_group_id=?")
            .bind(installed.group_id)
            .fetch_one(&b)
            .await
            .unwrap();
        assert_eq!(members, 1);
        let assignments: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM models_labels WHERE model_id=?")
                .bind(target_model)
                .fetch_one(&b)
                .await
                .unwrap();
        assert_eq!(
            assignments, 1,
            "the user's restored label assignment must remain"
        );
    }
}
