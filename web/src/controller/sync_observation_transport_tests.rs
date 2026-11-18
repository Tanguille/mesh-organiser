//! The opt-in observation route must authenticate and verify logical blob bytes.
use std::io::{Read, Write};

use tokio::io::AsyncWriteExt;

use super::*;

fn no_stage_payload(path: &std::path::Path) -> bool {
    path.is_dir()
        && std::fs::read_dir(path).unwrap().all(|entry| {
            let entry = entry.unwrap();
            entry.file_name() == ".lease" && entry.file_type().unwrap().is_file()
        })
}

fn entries(path: &std::path::Path) -> Vec<String> {
    let mut result: Vec<_> = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| {
            let name = entry.file_name();
            !(name.to_string_lossy().starts_with(".sync-stage-") && no_stage_payload(&entry.path()))
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    result.sort();
    result
}

#[tokio::test]
async fn oversized_observation_metadata_does_not_enroll_or_leave_staging() {
    let (source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('oversized','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(db::random_hex_32()).execute(&*source.db).await.unwrap().last_insert_rowid();
    let path = source_dir.path().join("large-metadata.stl");
    std::fs::write(&path, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &source.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        Some(path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let model = db::model_db::add_model(&source.db, &user, "part", blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&source.db, &user, Some(group), vec![model], None)
        .await
        .unwrap();
    sqlx::query("UPDATE models SET model_desc=? WHERE model_id=?")
        .bind("x".repeat(1_100_000))
        .bind(model)
        .execute(&*source.db)
        .await
        .unwrap();
    let model_dir = source.get_model_dir();
    let files = entries(&model_dir);
    let before: (i64,i64,i64,i64,i64,Option<String>,Option<String>) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM entity_births),(SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT COUNT(*) FROM causal_observation_batches),(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT group_birth_id FROM models_group WHERE group_id=?),(SELECT model_birth_id FROM models WHERE model_id=?)").bind(group).bind(model).fetch_one(&*source.db).await.unwrap();
    let result =
        service::sync_observation_service::prepare_observed_group(&source, &user, &identity, group)
            .await;
    assert!(
        result.is_err(),
        "metadata above the transport limit must be rejected"
    );
    drop(result);
    let after: (i64,i64,i64,i64,i64,Option<String>,Option<String>) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM entity_births),(SELECT COUNT(*) FROM causal_events),(SELECT COUNT(*) FROM causal_genesis_bindings),(SELECT COUNT(*) FROM causal_observation_batches),(SELECT counter FROM group_causal_replica WHERE singleton=1),(SELECT group_birth_id FROM models_group WHERE group_id=?),(SELECT model_birth_id FROM models WHERE model_id=?)").bind(group).bind(model).fetch_one(&*source.db).await.unwrap();
    assert_eq!(
        after, before,
        "oversized transport metadata must not enroll or advance the clock"
    );
    service::sync_observation_service::recover_transfers(&source)
        .await
        .unwrap();
    assert_eq!(
        entries(&model_dir),
        files,
        "rejected preparation must remove staging files"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines, clippy::significant_drop_tightening)] // The staged snapshot must live through upload and retry.
async fn compressed_blob_upload_and_download_verify_logical_content() {
    const FULL_HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let (_hub_dir, hub, router, cookie) = app().await;
    let (source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &source).await;
    db::sync_peer::bind_peer(&source.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    let global = db::random_hex_32();
    let group = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('zip','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(global).execute(&*source.db).await.unwrap().last_insert_rowid();
    let zip_path = source
        .get_model_dir()
        .join(format!("{}.stl.zip", &FULL_HASH[..32]));
    let mut archive = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
    archive
        .start_file(
            "part.stl",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
    archive.write_all(b"abc").unwrap();
    archive.finish().unwrap();
    let blob = db::blob_db::add_blob(&source.db, &FULL_HASH[..32], "stl.zip", 3, None)
        .await
        .unwrap();
    let model = db::model_db::add_model(&source.db, &user, "part", blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&source.db, &user, Some(group), vec![model], None)
        .await
        .unwrap();
    let prepared =
        service::sync_observation_service::prepare_observed_group(&source, &user, &identity, group)
            .await
            .unwrap();
    let original_item = prepared.item().clone();
    let original_event = prepared.group_event().clone();
    let event_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events")
        .fetch_one(&*source.db)
        .await
        .unwrap();
    assert_eq!(prepared.blobs()[0].key(), FULL_HASH);
    assert_eq!(
        std::fs::read(prepared.blobs()[0].path()).unwrap(),
        b"abc",
        "staging contains the logical file, not ZIP container bytes"
    );
    let proof =
        db::sync_transport::writer_proof(&source.db, &user, &identity, prepared.group_event())
            .await
            .unwrap();
    let payload = serde_json::json!({"protocol":1,"connection_id":proof.connection_id,"writer_nonce":proof.writer_nonce,"item":prepared.item()});
    let before: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM sync_transport_log)").fetch_one(&*hub.db).await.unwrap();
    for field in ["label_snapshot", "project_snapshot"] {
        assert!(payload["item"][field].is_null());
        let mut malformed = payload.clone();
        malformed["item"].as_object_mut().unwrap().remove(field);
        let response = router
            .clone()
            .oneshot(multipart(
                &malformed,
                &[(FULL_HASH, b"abc")],
                Some(&cookie),
                Some(&writer.writer_secret),
            ))
            .await
            .unwrap();
        assert_ne!(
            response.status(),
            StatusCode::OK,
            "omitted {field} cannot be acknowledged if receipt reconstruction inserts it"
        );
        let mut wrong_type = payload.clone();
        wrong_type["item"][field] = serde_json::json!(123);
        let response = router
            .clone()
            .oneshot(multipart(
                &wrong_type,
                &[(FULL_HASH, b"abc")],
                Some(&cookie),
                Some(&writer.writer_secret),
            ))
            .await
            .unwrap();
        assert_ne!(
            response.status(),
            StatusCode::OK,
            "{field} with the wrong type cannot be admitted"
        );
    }
    let mut unknown = payload.clone();
    unknown["item"]["unknown_field"] = serde_json::json!(null);
    let response = router
        .clone()
        .oneshot(multipart(
            &unknown,
            &[(FULL_HASH, b"abc")],
            Some(&cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_ne!(
        response.status(),
        StatusCode::OK,
        "unknown wire fields cannot be silently dropped"
    );
    let after: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM sync_transport_log)").fetch_one(&*hub.db).await.unwrap();
    assert_eq!(after, before);
    let response = router
        .clone()
        .oneshot(multipart(
            &payload,
            &[(FULL_HASH, b"abc")],
            Some(&cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let result: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    let sequence = result["sequence"].as_str().unwrap();
    drop(prepared);
    source.db.close().await;
    let reopened = db::db_context::setup_db(
        &source_dir.path().join("db.sqlite"),
        &source_dir.path().join("backups"),
    )
    .await;
    let mut resumed = source.clone();
    resumed.db = std::sync::Arc::new(reopened);
    let clock_before: (String, i64) =
        sqlx::query_as("SELECT writer_id,counter FROM causal_account_writers WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*resumed.db)
            .await
            .unwrap();
    let prepared = service::sync_observation_service::prepare_observed_group(
        &resumed, &user, &identity, group,
    )
    .await;
    assert!(
        prepared.is_ok(),
        "an unchanged observation must be re-preparable after restart: {prepared:?}"
    );
    let prepared = prepared.unwrap();
    assert_eq!(
        prepared.item(),
        &original_item,
        "reprepare must preserve original metadata and event dots"
    );
    assert_eq!(prepared.group_event(), &original_event);
    let proof = db::sync_transport::writer_proof(&resumed.db, &user, &identity, &original_event)
        .await
        .unwrap();
    let retry_payload = serde_json::json!({"protocol":1,"connection_id":proof.connection_id,"writer_nonce":proof.writer_nonce,"item":prepared.item()});
    let retry = router
        .clone()
        .oneshot(multipart(
            &retry_payload,
            &[(FULL_HASH, b"abc")],
            Some(&cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::OK);
    let retry: serde_json::Value =
        serde_json::from_slice(&to_bytes(retry.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(retry["status"], "already_present");
    assert_eq!(retry["sequence"], sequence);
    let after_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events")
        .fetch_one(&*resumed.db)
        .await
        .unwrap();
    let clock_after: (String, i64) =
        sqlx::query_as("SELECT writer_id,counter FROM causal_account_writers WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*resumed.db)
            .await
            .unwrap();
    assert_eq!(after_count, event_count);
    assert_eq!(
        clock_after, clock_before,
        "reprepare must not allocate a new event"
    );
    sqlx::query("UPDATE models SET model_name='changed' WHERE model_id=?")
        .bind(model)
        .execute(&*resumed.db)
        .await
        .unwrap();
    sqlx::query("UPDATE models SET model_name='part' WHERE model_id=?")
        .bind(model)
        .execute(&*resumed.db)
        .await
        .unwrap();
    assert!(
        service::sync_observation_service::prepare_observed_group(
            &resumed, &user, &identity, group
        )
        .await
        .is_err(),
        "an ABA metadata edit must reject reprepare"
    );
    let request = Request::get(format!(
        "/api/v2/sync/observations/{sequence}/blobs/{FULL_HASH}?protocol=1&connection_id={}",
        writer.connection_id
    ))
    .header(header::COOKIE, &cookie)
    .header("x-sync-writer-secret", &writer.writer_secret)
    .body(Body::empty())
    .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        &to_bytes(response.into_body(), 8192).await.unwrap()[..],
        b"abc"
    );
    let file = entries(&hub.get_model_dir())
        .into_iter()
        .find(|name| name.starts_with(".sync-blob-"))
        .unwrap();
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(hub.get_model_dir().join(file)).unwrap()).unwrap();
    let mut decoded = Vec::new();
    archive
        .by_index(0)
        .unwrap()
        .read_to_end(&mut decoded)
        .unwrap();
    assert_eq!(
        decoded, b"abc",
        "managed compressed content must decode to verified bytes"
    );
}

struct InterruptedBody {
    bytes: Vec<u8>,
    offset: usize,
}

impl tokio::io::AsyncRead for InterruptedBody {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.offset == self.bytes.len() {
            return std::task::Poll::Ready(Err(std::io::Error::other("interrupted upload")));
        }
        let end = self
            .offset
            .saturating_add(buffer.remaining())
            .min(self.bytes.len());
        buffer.put_slice(&self.bytes[self.offset..end]);
        self.offset = end;
        std::task::Poll::Ready(Ok(()))
    }
}

fn multipart(
    request: &serde_json::Value,
    blobs: &[(&str, &[u8])],
    cookie: Option<&str>,
    secret: Option<&str>,
) -> Request<Body> {
    let boundary = "mesh-observation-test-boundary";
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"request\"\r\nContent-Type: application/json\r\n\r\n{request}\r\n").into_bytes();
    for (name, bytes) in blobs {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"blob.{name}\"; filename=\"data\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    let mut builder = Request::post("/api/v2/sync/observations").header(
        header::CONTENT_TYPE,
        format!("multipart/form-data; boundary={boundary}"),
    );
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(secret) = secret {
        builder = builder.header("x-sync-writer-secret", secret);
    }
    let stream = tokio_util::io::ReaderStream::with_capacity(std::io::Cursor::new(body), 4096);
    builder.body(Body::from_stream(stream)).unwrap()
}

#[tokio::test]
#[allow(clippy::too_many_lines, clippy::significant_drop_tightening)] // Keep the source snapshot alive across the transfer.
async fn trickling_upload_is_cut_off_by_the_total_receive_deadline() {
    const FULL_HASH: &str = "c4a700f85b7e9e5cdbdc51170409ee2ad48bebe2f2f0957a067937531a0a3c42";
    // Large enough that the trickled tail still contains many separate data chunks.
    const CONTENT: [u8; 2000] = [b'a'; 2000];
    const PREFIX: &str = "c4a700f85b7e9e5cdbdc51170409ee2a";
    let (_hub_dir, hub, router, cookie) = app().await;
    let (source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &source).await;
    db::sync_peer::bind_peer(&source.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    let group = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('trickle','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(db::random_hex_32()).execute(&*source.db).await.unwrap().last_insert_rowid();
    let path = source_dir.path().join("trickle.stl");
    std::fs::write(&path, CONTENT).unwrap();
    let blob = db::blob_db::add_blob(
        &source.db,
        PREFIX,
        "stl",
        2000,
        Some(path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let model = db::model_db::add_model(&source.db, &user, "part", blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&source.db, &user, Some(group), vec![model], None)
        .await
        .unwrap();
    let prepared =
        service::sync_observation_service::prepare_observed_group(&source, &user, &identity, group)
            .await
            .unwrap();
    let proof =
        db::sync_transport::writer_proof(&source.db, &user, &identity, prepared.group_event())
            .await
            .unwrap();
    let payload = serde_json::json!({"protocol":1,"connection_id":proof.connection_id,"writer_nonce":proof.writer_nonce,"item":prepared.item()});
    let complete = multipart(
        &payload,
        &[(FULL_HASH, &CONTENT[..])],
        Some(&cookie),
        Some(&writer.writer_secret),
    );
    let (parts, body) = complete.into_parts();
    let bytes = to_bytes(body, 1024 * 1024).await.unwrap();
    let (mut sender, receiver) = tokio::io::duplex(8192);
    let trickled = Request::from_parts(
        parts,
        Body::from_stream(tokio_util::io::ReaderStream::new(receiver)),
    );
    let before_files = entries(&hub.get_model_dir());
    let inflight = tokio::spawn(router.clone().oneshot(trickled));
    let tail = bytes.len() - 300;
    sender.write_all(&bytes[..tail]).await.unwrap();
    // Paused only after setup so real sqlite/axum work above is not racing virtual time.
    tokio::time::pause();
    // Virtual time jumps whenever the runtime is idle, including while the handler waits on
    // sqlite, so only start trickling once the receiver has created its stage. Millisecond
    // steps keep the virtual time spent here negligible.
    let mut polls = 0;
    while !entries(&hub.get_model_dir())
        .iter()
        .any(|name| name.starts_with(".sync-stage-"))
    {
        polls += 1;
        assert!(
            polls < 10_000,
            "receiver must start staging the partial stream"
        );
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    // Every data chunk arrives within the idle timeout, so only the total deadline (idle
    // allowance plus a rounded-down second for 2000 declared bytes) can cut this off early.
    let started = tokio::time::Instant::now();
    for byte in &bytes[tail..] {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        if sender.write_all(&[*byte]).await.is_err() {
            break;
        }
    }
    let elapsed = started.elapsed();
    drop(sender);
    let result = inflight.await.unwrap().unwrap();
    assert_eq!(
        result.status(),
        StatusCode::REQUEST_TIMEOUT,
        "a trickling upload must hit the total receive deadline"
    );
    // Without a total deadline the idle timeout only fires after the whole trickled tail
    // (hundreds of 20s gaps) because every data chunk resets it, and the status is the same 408.
    assert!(
        elapsed < std::time::Duration::from_secs(60),
        "the cut-off must come from the total deadline, not the idle timeout: {elapsed:?}"
    );
    // Virtual time must not run against sqlx's pool acquire timeout.
    tokio::time::resume();
    service::sync_observation_service::recover_transfers(&hub)
        .await
        .unwrap();
    assert_eq!(
        entries(&hub.get_model_dir()),
        before_files,
        "a timed-out upload must clean its stage"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines, clippy::significant_drop_tightening)] // Keep the source snapshot alive across the transfer checks.
async fn observed_group_upload_verifies_bytes_and_installs_atomically() {
    const FULL_HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const PREFIX: &str = "ba7816bf8f01cfea414140de5dae2223";
    const OTHER_HASH: &str = "cb8379ac2098aa165029e3938a51da0bcecfc008fd6795f401178647f96c5b34";
    let (hub_dir, hub, router, cookie) = app().await;
    let (source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &source).await;
    db::sync_peer::bind_peer(&source.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    let global = db::random_hex_32();
    let group = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('edited','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(&global).execute(&*source.db).await.unwrap().last_insert_rowid();
    let path = source_dir.path().join("logical.stl");
    std::fs::write(&path, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &source.db,
        PREFIX,
        "stl",
        3,
        Some(path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let model = db::model_db::add_model(&source.db, &user, "part", blob, None, None)
        .await
        .unwrap();
    db::group_db::set_group_id_on_models(&source.db, &user, Some(group), vec![model], None)
        .await
        .unwrap();
    let project = db::resource_db::add_resource(&source.db, &user, "project", None)
        .await
        .unwrap();
    db::resource_db::set_resource_on_group(&source.db, &user, Some(project.id), group, None)
        .await
        .unwrap();
    let label = db::label_db::add_label(&source.db, &user, "label", 13, None)
        .await
        .unwrap();
    db::label_db::add_labels_on_models(&source.db, &user, &[label.id], &[model], None)
        .await
        .unwrap();
    db::model_db::trash_models(&source.db, &user, &[model])
        .await
        .unwrap();
    let deleted: String = sqlx::query_scalar("SELECT model_deleted FROM models WHERE model_id=?")
        .bind(model)
        .fetch_one(&*source.db)
        .await
        .unwrap();
    let source_files = entries(&source.get_model_dir());
    let bindings_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_genesis_bindings")
        .fetch_one(&*source.db)
        .await
        .unwrap();
    std::fs::write(&path, b"abd").unwrap();
    let mismatch =
        service::sync_observation_service::prepare_observed_group(&source, &user, &identity, group)
            .await;
    assert!(
        mismatch.is_err(),
        "source bytes must match the legacy digest before enrollment"
    );
    let bindings_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_genesis_bindings")
        .fetch_one(&*source.db)
        .await
        .unwrap();
    assert_eq!(bindings_after, bindings_before);
    drop(mismatch);
    service::sync_observation_service::recover_transfers(&source)
        .await
        .unwrap();
    assert_eq!(
        entries(&source.get_model_dir()),
        source_files,
        "failed preparation must clean source staging"
    );
    std::fs::write(&path, b"abc").unwrap();
    let prepared =
        service::sync_observation_service::prepare_observed_group(&source, &user, &identity, group)
            .await;
    assert!(
        prepared.is_ok(),
        "a real edited group with verified logical bytes must be preparable: {prepared:?}"
    );
    let prepared = prepared.unwrap();
    assert_eq!(prepared.blobs().len(), 1);
    assert_eq!(prepared.blobs()[0].key(), FULL_HASH);
    assert_eq!(prepared.blobs()[0].size(), 3);
    assert_eq!(std::fs::read(prepared.blobs()[0].path()).unwrap(), b"abc");
    let staging = prepared.blobs()[0].path().parent().unwrap().to_path_buf();
    assert!(
        staging
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".sync-stage-")
    );
    std::fs::write(&path, b"abd").unwrap();
    assert_eq!(
        std::fs::read(prepared.blobs()[0].path()).unwrap(),
        b"abc",
        "a staged transfer must not change when the source file changes"
    );
    std::fs::write(&path, b"abc").unwrap();
    assert!(
        !prepared.item().to_string().contains(path.to_str().unwrap()),
        "source paths cannot enter transport metadata"
    );
    let proof =
        db::sync_transport::writer_proof(&source.db, &user, &identity, prepared.group_event())
            .await
            .unwrap();
    let payload = serde_json::json!({"protocol":1,"connection_id":proof.connection_id,"writer_nonce":proof.writer_nonce,"item":prepared.item()});
    let before: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM blobs),(SELECT COUNT(*) FROM sync_transport_log)").fetch_one(&*hub.db).await.unwrap();
    let model_dir = hub.get_model_dir();
    let before_files = entries(&model_dir);
    // Hold the multipart stream open after the receiver creates its stage.
    // Ordinary imports must remain able to acquire their shared lock.
    let complete = multipart(
        &payload,
        &[(FULL_HASH, b"abc")],
        Some(&cookie),
        Some(&writer.writer_secret),
    );
    let (parts, body) = complete.into_parts();
    let bytes = to_bytes(body, 1024 * 1024).await.unwrap();
    let (mut sender, receiver) = tokio::io::duplex(8192);
    let stalled = Request::from_parts(
        parts,
        Body::from_stream(tokio_util::io::ReaderStream::new(receiver)),
    );
    let inflight = tokio::spawn(router.clone().oneshot(stalled));
    sender.write_all(&bytes[..bytes.len() - 8]).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !entries(&model_dir)
            .iter()
            .any(|name| name.starts_with(".sync-stage-"))
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("receiver must start staging the partial stream");
    let available = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        hub.import_mutex.lock(),
    )
    .await
    .is_ok();
    drop(sender);
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), inflight)
        .await
        .expect("interrupted upload must finish")
        .unwrap()
        .unwrap();
    assert_ne!(result.status(), StatusCode::OK);
    assert!(
        available,
        "a stalled remote stream must not hold the ordinary import lock"
    );
    service::sync_observation_service::recover_transfers(&hub)
        .await
        .unwrap();
    assert_eq!(
        entries(&model_dir),
        before_files,
        "a cancelled stream must clean its stage"
    );
    for (name, content, auth) in [
        (FULL_HASH, b"abd".as_slice(), true),
        (FULL_HASH, b"abc".as_slice(), false),
        ("bad", b"abc".as_slice(), true),
    ] {
        let response = router
            .clone()
            .oneshot(multipart(
                &payload,
                &[(name, content)],
                auth.then_some(cookie.as_str()),
                auth.then_some(writer.writer_secret.as_str()),
            ))
            .await
            .unwrap();
        assert_ne!(
            response.status(),
            StatusCode::OK,
            "bad bytes or missing auth cannot install"
        );
    }
    let wrong_secret = "0".repeat(64);
    let denied = router
        .clone()
        .oneshot(multipart(
            &payload,
            &[(FULL_HASH, b"abc")],
            Some(&cookie),
            Some(&wrong_secret),
        ))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "a session without the writer credential cannot publish"
    );
    let password = db::random_hex_32();
    db::user_db::add_user(&hub.db, "bob", "observation-bob@example.test", &password)
        .await
        .unwrap();
    let credentials = serde_json::json!({"email":"observation-bob@example.test","password":password,"group_deletion_protocol":1});
    let login = router
        .clone()
        .oneshot(post("/api/v1/login/password", &credentials, None, None))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::NO_CONTENT);
    let bob_cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let bob_writer = registration(&router, &bob_cookie, &source).await;
    let mut bob_payload = payload.clone();
    bob_payload["connection_id"] = bob_writer.connection_id.into();
    let denied = router
        .clone()
        .oneshot(multipart(
            &bob_payload,
            &[(FULL_HASH, b"abc")],
            Some(&bob_cookie),
            Some(&bob_writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "another authenticated account cannot publish the owner's observation"
    );
    let missing = router
        .clone()
        .oneshot(multipart(
            &payload,
            &[],
            Some(&cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_ne!(missing.status(), StatusCode::OK);
    let duplicate = router
        .clone()
        .oneshot(multipart(
            &payload,
            &[(FULL_HASH, b"abc"), (FULL_HASH, b"abc")],
            Some(&cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_ne!(duplicate.status(), StatusCode::OK);
    let extra = router
        .clone()
        .oneshot(multipart(
            &payload,
            &[(FULL_HASH, b"abc"), (&"0".repeat(64), b"abc")],
            Some(&cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_ne!(
        extra.status(),
        StatusCode::OK,
        "unrequested extra blobs cannot be installed"
    );
    let oversized = vec![0_u8; 32 * 1024 * 1024 + 1];
    let rejected = router
        .clone()
        .oneshot(multipart(
            &payload,
            &[(FULL_HASH, &oversized)],
            Some(&cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let complete = multipart(
        &payload,
        &[(FULL_HASH, b"abc")],
        Some(&cookie),
        Some(&writer.writer_secret),
    );
    let (parts, body) = complete.into_parts();
    let bytes = to_bytes(body, 1024 * 1024).await.unwrap();
    let cut = bytes.len() - 8;
    let reader = InterruptedBody {
        bytes: bytes[..cut].to_vec(),
        offset: 0,
    };
    let interrupted = Request::from_parts(
        parts,
        Body::from_stream(tokio_util::io::ReaderStream::new(reader)),
    );
    let response = router.clone().oneshot(interrupted).await.unwrap();
    assert_ne!(
        response.status(),
        StatusCode::OK,
        "an interrupted stream cannot be acknowledged"
    );
    let after: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM blobs),(SELECT COUNT(*) FROM sync_transport_log)").fetch_one(&*hub.db).await.unwrap();
    assert_eq!(
        after, before,
        "failed uploads cannot leave partial database state"
    );
    service::sync_observation_service::recover_transfers(&hub)
        .await
        .unwrap();
    assert_eq!(
        entries(&model_dir),
        before_files,
        "failed uploads cannot leave staged or final files"
    );
    let collision_path = hub_dir.path().join("existing.stl");
    std::fs::write(&collision_path, b"abd").unwrap();
    let collision = db::blob_db::add_blob(
        &hub.db,
        PREFIX,
        "stl",
        3,
        Some(collision_path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let hub_owner = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let existing_model =
        db::model_db::add_model(&hub.db, &hub_owner, "existing", collision, None, None)
            .await
            .unwrap();
    db::model_db::trash_models(&hub.db, &hub_owner, &[existing_model])
        .await
        .unwrap();
    let original_trash: String =
        sqlx::query_scalar("SELECT model_deleted FROM models WHERE model_id=?")
            .bind(existing_model)
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    let rejected = router
        .clone()
        .oneshot(multipart(
            &payload,
            &[(FULL_HASH, b"abc")],
            Some(&cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_ne!(
        rejected.status(),
        StatusCode::OK,
        "a legacy prefix match cannot substitute different bytes"
    );
    assert_eq!(
        std::fs::read(&collision_path).unwrap(),
        b"abd",
        "the existing file must remain untouched"
    );
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs WHERE blob_id=?")
        .bind(collision)
        .fetch_one(&*hub.db)
        .await
        .unwrap();
    assert_eq!(remaining, 1);
    let still_trashed: Option<String> =
        sqlx::query_scalar("SELECT model_deleted FROM models WHERE model_id=?")
            .bind(existing_model)
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    assert_eq!(
        still_trashed.as_deref(),
        Some(original_trash.as_str()),
        "upload must not untrash an unrelated model"
    );
    sqlx::query("DELETE FROM models WHERE model_id=?")
        .bind(existing_model)
        .execute(&*hub.db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM blobs WHERE blob_id=?")
        .bind(collision)
        .execute(&*hub.db)
        .await
        .unwrap();
    std::fs::remove_file(&collision_path).unwrap();
    sqlx::query("CREATE TRIGGER reject_observation_log BEFORE INSERT ON sync_transport_log BEGIN SELECT RAISE(ABORT,'injected observation log failure'); END").execute(&*hub.db).await.unwrap();
    let failure = router
        .clone()
        .oneshot(multipart(
            &payload,
            &[(FULL_HASH, b"abc")],
            Some(&cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_ne!(
        failure.status(),
        StatusCode::OK,
        "failed receipt cannot acknowledge install"
    );
    let after_failure: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM blobs),(SELECT COUNT(*) FROM sync_transport_log)").fetch_one(&*hub.db).await.unwrap();
    assert_eq!(
        after_failure, before,
        "receipt failure must roll back every installed row"
    );
    service::sync_observation_service::recover_transfers(&hub)
        .await
        .unwrap();
    assert_eq!(
        entries(&model_dir),
        before_files,
        "a rolled-back receipt must remove staged files"
    );
    sqlx::query("DROP TRIGGER reject_observation_log")
        .execute(&*hub.db)
        .await
        .unwrap();
    let response = router
        .clone()
        .oneshot(multipart(
            &payload,
            &[(FULL_HASH, b"abc")],
            Some(&cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    let status = response.status();
    let response_body = to_bytes(response.into_body(), 8192).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "verified upload must install: {}",
        String::from_utf8_lossy(&response_body)
    );
    let committed_files = entries(&model_dir);
    assert_eq!(committed_files.len(), before_files.len() + 1);
    assert!(
        committed_files
            .iter()
            .any(|name| name.starts_with(".sync-blob-"))
    );
    assert!(
        !committed_files
            .iter()
            .any(|name| name.starts_with(".sync-stage-"))
    );
    let retry = router
        .clone()
        .oneshot(multipart(
            &payload,
            &[(FULL_HASH, b"abc")],
            Some(&cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::OK);
    let retry: serde_json::Value =
        serde_json::from_slice(&to_bytes(retry.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(retry["status"], "already_present");
    let sequence = retry["sequence"].as_str().unwrap();
    let path = format!(
        "/api/v2/sync/observations/{sequence}/blobs/{FULL_HASH}?protocol=1&connection_id={}",
        writer.connection_id
    );
    let request = Request::get(&path)
        .header(header::COOKIE, &cookie)
        .header("x-sync-writer-secret", &writer.writer_secret)
        .body(Body::empty())
        .unwrap();
    let download = router.clone().oneshot(request).await.unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(
        &to_bytes(download.into_body(), 8192).await.unwrap()[..],
        b"abc",
        "download returns the verified logical bytes"
    );
    let request = Request::get(&path)
        .header("x-sync-writer-secret", &writer.writer_secret)
        .body(Body::empty())
        .unwrap();
    let denied = router.clone().oneshot(request).await.unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "writer secret without an authenticated session cannot download"
    );
    let bob_connection = bob_payload["connection_id"].as_str().unwrap();
    let other_path = format!(
        "/api/v2/sync/observations/{sequence}/blobs/{FULL_HASH}?protocol=1&connection_id={bob_connection}"
    );
    let request = Request::get(other_path)
        .header(header::COOKIE, &bob_cookie)
        .header("x-sync-writer-secret", &bob_writer.writer_secret)
        .body(Body::empty())
        .unwrap();
    let denied = router.clone().oneshot(request).await.unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "another account cannot download an owner's blob"
    );
    let wrong_path = format!(
        "/api/v2/sync/observations/{sequence}/blobs/{}?protocol=1&connection_id={}",
        "0".repeat(64),
        writer.connection_id
    );
    let request = Request::get(wrong_path)
        .header(header::COOKIE, &cookie)
        .header("x-sync-writer-secret", &writer.writer_secret)
        .body(Body::empty())
        .unwrap();
    let denied = router.clone().oneshot(request).await.unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "sequence cannot authorize an unrelated blob key"
    );
    let committed_name = committed_files
        .iter()
        .find(|name| name.starts_with(".sync-blob-"))
        .unwrap();
    let committed_path = model_dir.join(committed_name);
    std::fs::write(&committed_path, b"abd").unwrap();
    let request = Request::get(&path)
        .header(header::COOKIE, &cookie)
        .header("x-sync-writer-secret", &writer.writer_secret)
        .body(Body::empty())
        .unwrap();
    let denied = router.clone().oneshot(request).await.unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "modified committed content cannot be downloaded as verified bytes"
    );
    std::fs::write(&committed_path, b"abc").unwrap();
    service::sync_observation_service::recover_transfers(&hub)
        .await
        .unwrap();
    assert_eq!(
        entries(&model_dir),
        committed_files,
        "retry must not create another file"
    );
    let owner = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let row: (String,Option<String>,String,String) = sqlx::query_as("SELECT m.model_name,m.model_deleted,r.resource_name,l.label_name FROM models m JOIN models_group g ON g.group_id=m.model_group_id JOIN resources r ON r.resource_id=g.group_resource_id JOIN models_labels ml ON ml.model_id=m.model_id JOIN labels l ON l.label_id=ml.label_id WHERE m.model_user_id=? AND g.group_unique_global_id=?").bind(owner.id).bind(&global).fetch_one(&*hub.db).await.unwrap();
    assert_eq!(
        row,
        (
            "part".into(),
            Some(deleted),
            "project".into(),
            "label".into()
        )
    );
    // A second writer owns a different object with the same global group id.
    // It must not merge into or replace the already-installed group.
    let (other_dir, other) = crate::test_support::app_state().await;
    let other_identity = db::user_db::get_sync_identity(&other.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let other_writer = registration(&router, &cookie, &other).await;
    db::sync_peer::bind_peer(
        &other.db,
        &user,
        &other_identity,
        &other_writer.server_identity,
    )
    .await
    .unwrap();
    db::sync_transport::install_authority(&other.db, &user, &other_identity, &other_writer)
        .await
        .unwrap();
    let other_group = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('collision','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(&global).execute(&*other.db).await.unwrap().last_insert_rowid();
    let other_path = other_dir.path().join("collision.stl");
    std::fs::write(&other_path, b"def").unwrap();
    let other_blob = db::blob_db::add_blob(
        &other.db,
        &OTHER_HASH[..32],
        "stl",
        3,
        Some(other_path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let other_model =
        db::model_db::add_model(&other.db, &user, "collision", other_blob, None, None)
            .await
            .unwrap();
    db::group_db::set_group_id_on_models(
        &other.db,
        &user,
        Some(other_group),
        vec![other_model],
        None,
    )
    .await
    .unwrap();
    let other_prepared = service::sync_observation_service::prepare_observed_group(
        &other,
        &user,
        &other_identity,
        other_group,
    )
    .await
    .unwrap();
    let other_proof = db::sync_transport::writer_proof(
        &other.db,
        &user,
        &other_identity,
        other_prepared.group_event(),
    )
    .await
    .unwrap();
    let other_payload = serde_json::json!({"protocol":1,"connection_id":other_proof.connection_id,"writer_nonce":other_proof.writer_nonce,"item":other_prepared.item()});
    let before_collision: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM blobs),(SELECT COUNT(*) FROM sync_transport_log)").fetch_one(&*hub.db).await.unwrap();
    let collision = router
        .clone()
        .oneshot(multipart(
            &other_payload,
            &[(OTHER_HASH, b"def")],
            Some(&cookie),
            Some(&other_writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_ne!(
        collision.status(),
        StatusCode::OK,
        "occupied group identity cannot be merged"
    );
    let after_collision: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM blobs),(SELECT COUNT(*) FROM sync_transport_log)").fetch_one(&*hub.db).await.unwrap();
    assert_eq!(after_collision, before_collision);
    service::sync_observation_service::recover_transfers(&hub)
        .await
        .unwrap();
    assert_eq!(entries(&model_dir), committed_files);
    assert_eq!(std::fs::read(&other_path).unwrap(), b"def");
    drop(other_prepared);
    let external_path = hub_dir.path().join("keep-external.stl");
    std::fs::write(&external_path, b"external").unwrap();
    db::blob_db::add_blob(
        &hub.db,
        &db::random_hex_32(),
        "stl",
        8,
        Some(external_path.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(
        db::model_db::purge_trashed_models(&hub.db, 0)
            .await
            .unwrap(),
        1
    );
    service::export_service::delete_dead_blobs(&hub)
        .await
        .unwrap();
    assert!(
        !committed_path.exists(),
        "expired app-owned transferred bytes must be removed"
    );
    assert_eq!(
        std::fs::read(&external_path).unwrap(),
        b"external",
        "GC must preserve external originals"
    );
    let receipt: (String, Option<i64>) = sqlx::query_as(
        "SELECT sha256,blob_id FROM sync_observation_contents WHERE user_id=? AND sha256=?",
    )
    .bind(owner.id)
    .bind(FULL_HASH)
    .fetch_one(&*hub.db)
    .await
    .unwrap();
    assert_eq!(
        receipt,
        (FULL_HASH.into(), None),
        "purge retains immutable receipt metadata without retaining expired blob"
    );
    let request = Request::get(&path)
        .header(header::COOKIE, &cookie)
        .header("x-sync-writer-secret", &writer.writer_secret)
        .body(Body::empty())
        .unwrap();
    let denied = router.clone().oneshot(request).await.unwrap();
    assert_ne!(
        denied.status(),
        StatusCode::OK,
        "expired content cannot be downloaded"
    );
    let listing = format!(
        "/api/v2/sync/items?protocol=1&connection_id={}&after=0&limit=64",
        writer.connection_id
    );
    let request = Request::get(listing)
        .header(header::COOKIE, &cookie)
        .header("x-sync-writer-secret", &writer.writer_secret)
        .body(Body::empty())
        .unwrap();
    let page = router.clone().oneshot(request).await.unwrap();
    assert_eq!(
        page.status(),
        StatusCode::OK,
        "receipt metadata remains readable after content expires"
    );
    db::user_db::delete_user(&hub.db, owner.id).await.unwrap();
    assert_eq!(
        std::fs::read(&external_path).unwrap(),
        b"external",
        "account deletion must not remove external originals"
    );
    sqlx::query("UPDATE labels SET label_name='changed' WHERE label_id=?")
        .bind(label.id)
        .execute(&*source.db)
        .await
        .unwrap();
    sqlx::query("UPDATE labels SET label_name='label' WHERE label_id=?")
        .bind(label.id)
        .execute(&*source.db)
        .await
        .unwrap();
    assert!(
        service::sync_observation_service::prepare_observed_group(&source, &user, &identity, group)
            .await
            .is_err(),
        "a label ABA must reject re-preparation"
    );
    drop(prepared);
    service::sync_observation_service::recover_transfers(&source)
        .await
        .unwrap();
    assert!(
        !staging.exists() || no_stage_payload(&staging),
        "dropping the source transfer must clean app-owned staged content"
    );
}
