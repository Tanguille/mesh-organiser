//! Checked genesis is a narrow prerequisite, not existing-library or authenticated sync.
mod common;
use db::{
    blob_db, causal_genesis, group_db, model::user::User, model_db, random_hex_32, sync_peer,
    user_db,
};

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn independent_databases_install_original_group_and_model_genesis_idempotently() {
    let (_a_dir, a) = common::test_db().await;
    let (_b_dir, b) = common::test_db().await;
    let user = User::default();
    let aid = user_db::get_sync_identity(&a, user.id)
        .await
        .unwrap()
        .unwrap();
    let bid = user_db::get_sync_identity(&b, user.id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(aid, bid);
    let group = group_db::add_empty_group(&a, &user, "source", None)
        .await
        .unwrap();
    let hash = random_hex_32();
    let blob_a = blob_db::add_blob(&a, &hash, "stl", 123, None)
        .await
        .unwrap();
    let model = model_db::add_model(&a, &user, "part", blob_a, None, None)
        .await
        .unwrap();
    // Reserve different local ids independently, including a different blob id.
    group_db::add_empty_group(&b, &user, "unrelated", None)
        .await
        .unwrap();
    let other_blob = blob_db::add_blob(&b, &"f".repeat(32), "stl", 1, None)
        .await
        .unwrap();
    model_db::add_model(&b, &user, "unrelated", other_blob, None, None)
        .await
        .unwrap();
    let blob_b = blob_db::add_blob(&b, &hash, "stl", 123, None)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&b, &user, &bid, &aid).await.unwrap();
    let group_bundle = causal_genesis::enroll_pristine_group(&a, &user, &aid, group.id).await;
    assert!(
        group_bundle.is_ok(),
        "a freshly created unmodified group is enrollable"
    );
    let group_bundle = group_bundle.unwrap();
    let model_bundle = causal_genesis::enroll_pristine_model(&a, &user, &aid, model)
        .await
        .unwrap();
    assert_eq!(group_bundle.source_identity(), &aid);
    assert_eq!(model_bundle.source_identity(), &aid);
    let group_install =
        causal_genesis::install_pristine_genesis(&b, &user, &bid, &peer, &group_bundle, None)
            .await
            .unwrap();
    let model_install = causal_genesis::install_pristine_genesis(
        &b,
        &user,
        &bid,
        &peer,
        &model_bundle,
        Some(blob_b),
    )
    .await
    .unwrap();
    assert_ne!(group_install.local_id, group.id);
    assert_ne!(model_install.local_id, model);
    assert!(!group_install.already_present && !model_install.already_present);
    let (g_global, g_birth): (String, String) = sqlx::query_as(
        "SELECT group_unique_global_id,group_birth_id FROM models_group WHERE group_id=?",
    )
    .bind(group_install.local_id)
    .fetch_one(&b)
    .await
    .unwrap();
    assert_eq!(g_global, group_bundle.entity().global_id);
    assert_eq!(Some(&g_birth), group_bundle.entity().birth_id.as_ref());
    let (m_global, m_birth, m_blob, m_name, m_inc): (String, String, i64, String, String) = sqlx::query_as(
        "SELECT model_unique_global_id,model_birth_id,model_blob_id,model_name,model_incarnation FROM models WHERE model_id=?",
    )
    .bind(model_install.local_id)
    .fetch_one(&b)
    .await
    .unwrap();
    assert_eq!(m_global, model_bundle.entity().global_id);
    assert_eq!(Some(&m_birth), model_bundle.entity().birth_id.as_ref());
    assert_eq!(m_blob, blob_b);
    assert_eq!(m_name, "part");
    let installed_eligible: i64 =
        sqlx::query_scalar("SELECT model_genesis_unchanged FROM models WHERE model_id=?")
            .bind(model_install.local_id)
            .fetch_one(&b)
            .await
            .unwrap();
    assert_eq!(
        installed_eligible, 0,
        "a remotely installed model is not a locally pristine genesis"
    );
    assert!(
        causal_genesis::enroll_pristine_model(&b, &user, &bid, model_install.local_id)
            .await
            .is_err()
    );
    let source_inc: String =
        sqlx::query_scalar("SELECT model_incarnation FROM models WHERE model_id=?")
            .bind(model)
            .fetch_one(&a)
            .await
            .unwrap();
    assert_ne!(m_inc, source_inc);
    for event in [group_bundle.event(), model_bundle.event()] {
        let bytes_a: String = sqlx::query_scalar(
            "SELECT envelope FROM causal_events WHERE user_id=? AND writer_id=? AND counter=?",
        )
        .bind(user.id)
        .bind(&event.writer_id)
        .bind(event.counter.parse::<i64>().unwrap())
        .fetch_one(&a)
        .await
        .unwrap();
        let bytes_b: String = sqlx::query_scalar(
            "SELECT envelope FROM causal_events WHERE user_id=? AND writer_id=? AND counter=?",
        )
        .bind(user.id)
        .bind(&event.writer_id)
        .bind(event.counter.parse::<i64>().unwrap())
        .fetch_one(&b)
        .await
        .unwrap();
        assert_eq!(bytes_a, bytes_b);
    }
    let retry =
        causal_genesis::install_pristine_genesis(&b, &user, &bid, &peer, &group_bundle, None)
            .await
            .unwrap();
    assert!(retry.already_present);
    assert_eq!(retry.local_id, group_install.local_id);
}

#[tokio::test]
async fn edited_and_legacy_entities_cannot_be_enrolled_as_pristine() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let legacy = common::legacy_group(&db, &user, "legacy").await;
    assert!(
        causal_genesis::enroll_pristine_group(&db, &user, &identity, legacy.id)
            .await
            .is_err()
    );
    let group = group_db::add_empty_group(&db, &user, "before", None)
        .await
        .unwrap();
    let other_id = user_db::add_user(&db, "other", "genesis-other@example.test", &random_hex_32())
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
        causal_genesis::enroll_pristine_group(&db, &other, &other_identity, group.id)
            .await
            .is_err()
    );
    assert!(
        causal_genesis::enroll_pristine_group(&db, &user, &other_identity, group.id)
            .await
            .is_err()
    );
    group_db::edit_group(&db, &user, group.id, "after", None, None)
        .await
        .unwrap();
    assert!(
        causal_genesis::enroll_pristine_group(&db, &user, &identity, group.id)
            .await
            .is_err()
    );
    let hash = random_hex_32();
    let blob = blob_db::add_blob(&db, &hash, "stl", 10, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "model", blob, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    assert!(
        causal_genesis::enroll_pristine_model(&db, &user, &identity, model)
            .await
            .is_err()
    );
    let legacy_model = common::legacy_model(&db, &user, "legacy-model").await;
    assert!(
        causal_genesis::enroll_pristine_model(&db, &user, &identity, legacy_model)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn installation_rejects_wrong_peer_blob_and_rolls_back_failure() {
    let (_a_dir, a) = common::test_db().await;
    let (_b_dir, b) = common::test_db().await;
    let user = User::default();
    let aid = user_db::get_sync_identity(&a, user.id)
        .await
        .unwrap()
        .unwrap();
    let bid = user_db::get_sync_identity(&b, user.id)
        .await
        .unwrap()
        .unwrap();
    let hash = random_hex_32();
    let source_blob = blob_db::add_blob(&a, &hash, "stl", 12, None).await.unwrap();
    let model = model_db::add_model(&a, &user, "model", source_blob, None, None)
        .await
        .unwrap();
    let bundle = causal_genesis::enroll_pristine_model(&a, &user, &aid, model)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&b, &user, &bid, &aid).await.unwrap();
    assert!(
        causal_genesis::install_pristine_genesis(&b, &user, &bid, &peer, &bundle, None)
            .await
            .is_err()
    );
    let wrong_hash = blob_db::add_blob(&b, &random_hex_32(), "stl", 12, None)
        .await
        .unwrap();
    assert!(
        causal_genesis::install_pristine_genesis(&b, &user, &bid, &peer, &bundle, Some(wrong_hash))
            .await
            .is_err()
    );
    let wrong = blob_db::add_blob(&b, &hash, "obj", 12, None).await.unwrap();
    assert!(
        causal_genesis::install_pristine_genesis(&b, &user, &bid, &peer, &bundle, Some(wrong))
            .await
            .is_err()
    );
    sqlx::query("UPDATE blobs SET blob_filetype='stl', blob_size=13 WHERE blob_id=?")
        .bind(wrong)
        .execute(&b)
        .await
        .unwrap();
    assert!(
        causal_genesis::install_pristine_genesis(&b, &user, &bid, &peer, &bundle, Some(wrong))
            .await
            .is_err()
    );
    sqlx::query("UPDATE blobs SET blob_size=12 WHERE blob_id=?")
        .bind(wrong)
        .execute(&b)
        .await
        .unwrap();
    let good = wrong;
    let unbound = sync_peer::PeerBinding {
        local: bid.clone(),
        remote: bid.clone(),
    };
    assert!(
        causal_genesis::install_pristine_genesis(&b, &user, &bid, &unbound, &bundle, Some(good))
            .await
            .is_err()
    );
    sqlx::query("CREATE TRIGGER reject_genesis BEFORE INSERT ON causal_genesis_bindings BEGIN SELECT RAISE(ABORT,'injected genesis failure'); END").execute(&b).await.unwrap();
    let before: (i64, i64, i64, i64, i64) =
        sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM entity_births),(SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT highwater FROM object_id_highwater WHERE object_kind='model')")
            .fetch_one(&b)
            .await
            .unwrap();
    let error =
        causal_genesis::install_pristine_genesis(&b, &user, &bid, &peer, &bundle, Some(good))
            .await
            .unwrap_err();
    assert!(error.to_string().contains("injected genesis failure"));
    let after: (i64, i64, i64, i64, i64) =
        sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM entity_births),(SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT highwater FROM object_id_highwater WHERE object_kind='model')")
            .fetch_one(&b)
            .await
            .unwrap();
    assert_eq!(after, before);
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_identical_installs_create_one_group_and_retry_does_not_overwrite_edits() {
    let (_a_dir, a) = common::test_db().await;
    let (_b_dir, b) = common::test_db().await;
    let user = User::default();
    let aid = user_db::get_sync_identity(&a, user.id)
        .await
        .unwrap()
        .unwrap();
    let bid = user_db::get_sync_identity(&b, user.id)
        .await
        .unwrap()
        .unwrap();
    let source = group_db::add_empty_group(&a, &user, "original", None)
        .await
        .unwrap();
    let bundle = causal_genesis::enroll_pristine_group(&a, &user, &aid, source.id)
        .await
        .unwrap();
    let peer = sync_peer::bind_peer(&b, &user, &bid, &aid).await.unwrap();
    let (left, right) = tokio::join!(
        causal_genesis::install_pristine_genesis(&b, &user, &bid, &peer, &bundle, None),
        causal_genesis::install_pristine_genesis(&b, &user, &bid, &peer, &bundle, None)
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.local_id, right.local_id);
    assert_ne!(left.already_present, right.already_present);
    group_db::edit_group(&b, &user, left.local_id, "changed", None, None)
        .await
        .unwrap();
    let retry = causal_genesis::install_pristine_genesis(&b, &user, &bid, &peer, &bundle, None)
        .await
        .unwrap();
    assert!(retry.already_present);
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=?")
        .bind(left.local_id)
        .fetch_one(&b)
        .await
        .unwrap();
    assert_eq!(name, "changed");
}

#[tokio::test]
async fn stale_account_identity_cannot_install_after_numeric_user_id_reuse() {
    let (_a_dir, a) = common::test_db().await;
    let (_b_dir, b) = common::test_db().await;
    let source_user = User::default();
    let aid = user_db::get_sync_identity(&a, source_user.id)
        .await
        .unwrap()
        .unwrap();
    let source = group_db::add_empty_group(&a, &source_user, "source", None)
        .await
        .unwrap();
    let bundle = causal_genesis::enroll_pristine_group(&a, &source_user, &aid, source.id)
        .await
        .unwrap();
    let id = user_db::add_user(&b, "old", "genesis-old@example.test", &random_hex_32())
        .await
        .unwrap();
    let old = user_db::get_user_by_id(&b, id).await.unwrap().unwrap();
    let old_identity = user_db::get_sync_identity(&b, id).await.unwrap().unwrap();
    let peer = sync_peer::bind_peer(&b, &old, &old_identity, &aid)
        .await
        .unwrap();
    sqlx::query("DELETE FROM users WHERE user_id=?")
        .bind(id)
        .execute(&b)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (user_id,user_name,user_email,user_password_hash,user_created_at) VALUES (?,'new','genesis-new@example.test','unused','2020-01-01T00:00:00Z')").bind(id).execute(&b).await.unwrap();
    assert!(
        causal_genesis::install_pristine_genesis(&b, &old, &old_identity, &peer, &bundle, None)
            .await
            .is_err()
    );
    let groups: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models_group WHERE group_user_id=?")
        .bind(id)
        .fetch_one(&b)
        .await
        .unwrap();
    assert_eq!(groups, 0);
}

#[tokio::test]
async fn same_second_model_metadata_edit_and_revert_is_not_pristine() {
    use db::model::ModelFlags;
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "before", blob, None, None)
        .await
        .unwrap();
    let timestamp: String =
        sqlx::query_scalar("SELECT model_last_modified FROM models WHERE model_id=?")
            .bind(model)
            .fetch_one(&db)
            .await
            .unwrap();
    model_db::edit_model(
        &db,
        &user,
        model,
        "changed",
        None,
        None,
        ModelFlags::empty(),
        Some(&timestamp),
        None,
    )
    .await
    .unwrap();
    model_db::edit_model(
        &db,
        &user,
        model,
        "before",
        None,
        None,
        ModelFlags::empty(),
        Some(&timestamp),
        None,
    )
    .await
    .unwrap();
    assert!(
        causal_genesis::enroll_pristine_model(&db, &user, &identity, model)
            .await
            .is_err(),
        "same-second metadata ABA cannot be accepted as pristine"
    );
}

#[tokio::test]
async fn model_label_add_remove_is_not_pristine_even_with_identical_timestamps() {
    use db::label_db;
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "model", blob, None, None)
        .await
        .unwrap();
    let label = label_db::add_label(&db, &user, "label", 0, None)
        .await
        .unwrap();
    let timestamp: String =
        sqlx::query_scalar("SELECT model_last_modified FROM models WHERE model_id=?")
            .bind(model)
            .fetch_one(&db)
            .await
            .unwrap();
    label_db::add_labels_on_models(&db, &user, &[label.id], &[model], Some(&timestamp))
        .await
        .unwrap();
    label_db::remove_labels_from_models(&db, &user, &[label.id], &[model], Some(&timestamp))
        .await
        .unwrap();
    assert!(
        causal_genesis::enroll_pristine_model(&db, &user, &identity, model)
            .await
            .is_err(),
        "removed labels still prove unsupported prior changes"
    );
}

#[tokio::test]
async fn failed_model_edit_and_label_insert_leave_pristine_eligibility_intact() {
    use db::{label_db, model::ModelFlags};
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "model", blob, None, None)
        .await
        .unwrap();
    let label = label_db::add_label(&db, &user, "label", 0, None)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER reject_metadata_edit BEFORE UPDATE ON models WHEN NEW.model_name='changed' BEGIN SELECT RAISE(ABORT,'injected model error'); END").execute(&db).await.unwrap();
    let error = model_db::edit_model(
        &db,
        &user,
        model,
        "changed",
        None,
        None,
        ModelFlags::empty(),
        None,
        None,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("injected model error"));
    sqlx::query("CREATE TRIGGER reject_label_insert BEFORE INSERT ON models_labels BEGIN SELECT RAISE(ABORT,'injected label error'); END").execute(&db).await.unwrap();
    let error = label_db::add_labels_on_models(&db, &user, &[label.id], &[model], None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("injected label error"));
    assert!(
        causal_genesis::enroll_pristine_model(&db, &user, &identity, model)
            .await
            .is_ok(),
        "failed mutations must not invalidate pristine state"
    );
}

#[tokio::test]
async fn model_created_before_eligibility_migration_remains_ineligible() {
    use db::causal_envelope::{self, Action, CausalEnvelope, Dot, Entity, EntityKind};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let old = sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version < 20_261_113_140_000)
        .cloned()
        .collect();
    sqlx::migrate::Migrator::with_migrations(old)
        .run(&db)
        .await
        .unwrap();
    let user = User::default();
    let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 1, None)
        .await
        .unwrap();
    let global = random_hex_32();
    let birth = random_hex_32();
    let writer = random_hex_32();
    let id = sqlx::query("INSERT INTO models (model_id,model_name,model_blob_id,model_added,model_user_id,model_unique_global_id,model_last_modified,model_birth_id) VALUES (1,'old',?,'2020-01-01T00:00:00Z',?,?,'2020-01-01T00:00:00Z',?)").bind(blob).bind(user.id).bind(&global).bind(&birth).execute(&db).await.unwrap().last_insert_rowid();
    let event = CausalEnvelope {
        version: 2,
        entity: Entity {
            kind: EntityKind::Model,
            global_id: global,
            birth_id: Some(birth.clone()),
        },
        event: Dot {
            writer_id: writer.clone(),
            counter: "1".into(),
        },
        parents: vec![],
        dependencies: vec![],
        action: Action::ModelCreated,
    };
    let bytes = causal_envelope::canonical_causal_envelope(&event).unwrap();
    sqlx::query("INSERT INTO causal_events (user_id,writer_id,counter,envelope) VALUES (?,?,1,?)")
        .bind(user.id)
        .bind(&writer)
        .bind(bytes)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO local_creation_records (user_id,entity_kind,birth_id,writer_id,counter) VALUES (?,'model',?,?,1)").bind(user.id).bind(&birth).bind(&writer).execute(&db).await.unwrap();
    sqlx::query("UPDATE entity_births SET initialization_status='created',initialization_writer_id=?,initialization_counter=1 WHERE user_id=? AND entity_kind='model' AND birth_id=?").bind(&writer).bind(user.id).bind(&birth).execute(&db).await.unwrap();
    db.close().await;
    let db = db::db_context::setup_db(&path, &dir.path().join("backups")).await;
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        causal_genesis::enroll_pristine_model(&db, &user, &identity, id)
            .await
            .is_err(),
        "an old creation record cannot prove no metadata or label ABA occurred"
    );
}

#[tokio::test]
async fn shared_blob_descriptor_change_invalidates_models_across_owners_and_rollback_is_atomic() {
    let (_dir, db) = common::test_db().await;
    let first = User::default();
    let other_id = user_db::add_user(
        &db,
        "other",
        "shared-blob-other@example.test",
        &random_hex_32(),
    )
    .await
    .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let first_identity = user_db::get_sync_identity(&db, first.id)
        .await
        .unwrap()
        .unwrap();
    let other_identity = user_db::get_sync_identity(&db, other.id)
        .await
        .unwrap()
        .unwrap();
    let hash = random_hex_32();
    let blob = blob_db::add_blob(&db, &hash, "stl", 123, None)
        .await
        .unwrap();
    let a = model_db::add_model(&db, &first, "a", blob, None, None)
        .await
        .unwrap();
    let b = model_db::add_model(&db, &other, "b", blob, None, None)
        .await
        .unwrap();
    let mut tx = db.begin().await.unwrap();
    sqlx::query("UPDATE blobs SET blob_size=124 WHERE blob_id=?")
        .bind(blob)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let flags: Vec<i64> = sqlx::query_scalar(
        "SELECT model_genesis_unchanged FROM models WHERE model_id IN (?,?) ORDER BY model_id",
    )
    .bind(a)
    .bind(b)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(
        flags,
        vec![1, 1],
        "rolled-back descriptor change cannot invalidate either owner"
    );
    sqlx::query("UPDATE blobs SET blob_size=124 WHERE blob_id=?")
        .bind(blob)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE blobs SET blob_size=123 WHERE blob_id=?")
        .bind(blob)
        .execute(&db)
        .await
        .unwrap();
    let flags: Vec<i64> = sqlx::query_scalar(
        "SELECT model_genesis_unchanged FROM models WHERE model_id IN (?,?) ORDER BY model_id",
    )
    .bind(a)
    .bind(b)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(
        flags,
        vec![0, 0],
        "descriptor ABA invalidates every owner sharing the blob"
    );
    assert!(
        causal_genesis::enroll_pristine_model(&db, &first, &first_identity, a)
            .await
            .is_err()
    );
    assert!(
        causal_genesis::enroll_pristine_model(&db, &other, &other_identity, b)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn unchanged_model_and_blob_updates_preserve_pristine_eligibility() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let identity = user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let blob = blob_db::add_blob(&db, &random_hex_32(), "stl", 2, None)
        .await
        .unwrap();
    let model = model_db::add_model(&db, &user, "model", blob, None, None)
        .await
        .unwrap();
    sqlx::query("UPDATE models SET model_name=model_name,model_last_modified=model_last_modified,model_url=model_url,model_desc=model_desc,model_flags=model_flags WHERE model_id=?").bind(model).execute(&db).await.unwrap();
    sqlx::query("UPDATE blobs SET blob_sha256=blob_sha256,blob_filetype=blob_filetype,blob_size=blob_size WHERE blob_id=?").bind(blob).execute(&db).await.unwrap();
    let eligible: i64 =
        sqlx::query_scalar("SELECT model_genesis_unchanged FROM models WHERE model_id=?")
            .bind(model)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(eligible, 1);
    assert!(
        causal_genesis::enroll_pristine_model(&db, &user, &identity, model)
            .await
            .is_ok()
    );
}
