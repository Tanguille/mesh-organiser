//! Exercise the opt-in pull client over an actual authenticated TLS connection.
use std::{
    fmt::Write as _,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use axum::{
    extract::State,
    middleware::{self, Next},
    response::IntoResponse,
};
use sha2::Digest;
use tokio::sync::Notify;
use tokio_rustls::rustls;

use db::{
    causal_group_rename::{NameState, ResolveResult},
    causal_model_move::{MoveResolveResult, MoveState},
};
use service::sync_pull_service::{AuthenticatedHub, HubTrust, PullOutcome};

use super::*;

/// A throwaway CA plus a `localhost` leaf signed by it, generated once per
/// test process so no private key is committed.
struct TestTls {
    ca_pem: Vec<u8>,
    leaf_der: Vec<u8>,
    leaf_key_der: Vec<u8>,
}

fn test_tls() -> &'static TestTls {
    static TEST_TLS: std::sync::OnceLock<TestTls> = std::sync::OnceLock::new();

    TEST_TLS.get_or_init(|| {
        // Wide window: not-before in the past (clock skew) and not-after far ahead so CI
        // never races expiry.
        let validity = |params: &mut rcgen::CertificateParams| {
            params.not_before = rcgen::date_time_ymd(2020, 1, 1);
            params.not_after = rcgen::date_time_ymd(2100, 1, 1);
        };

        let mut ca_params = rcgen::CertificateParams::new(Vec::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "mesh-organiser test CA");
        validity(&mut ca_params);
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_key).unwrap();

        let mut leaf_params = rcgen::CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
        validity(&mut leaf_params);
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let issuer = rcgen::Issuer::new(ca_params, ca_key);
        let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();

        TestTls {
            ca_pem: ca.pem().into_bytes(),
            leaf_der: leaf.der().to_vec(),
            leaf_key_der: leaf_key.serialize_der(),
        }
    })
}

/// PEM of the generated test CA, for `HubTrust::AdditionalCaPem`.
pub(super) static TEST_CA: std::sync::LazyLock<Vec<u8>> =
    std::sync::LazyLock::new(|| test_tls().ca_pem.clone());

pub(super) struct TlsHub {
    address: std::net::SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TlsHub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TlsHub {
    pub(super) async fn serve(router: Router) -> Self {
        // Select the process-level CryptoProvider explicitly. Under feature
        // unification the test binary's rustls feature set can be ambiguous,
        // in which case `ServerConfig::builder()` below panics instead of
        // auto-detecting a provider. Installing it here (idempotently) is the
        // fix rustls recommends; the `aws-lc-rs` provider matches the rest of
        // the app (reqwest / tokio-rustls).
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let tls = test_tls();
        let cert = rustls::pki_types::CertificateDer::from(tls.leaf_der.clone());
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(tls.leaf_key_der.clone());
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key.into())
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let router = router.clone();
                tokio::spawn(async move {
                    if let Ok(stream) = acceptor.accept(stream).await {
                        let io = hyper_util::rt::TokioIo::new(stream);
                        let service = hyper_util::service::TowerToHyperService::new(router);
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(io, service)
                            .await;
                    }
                });
            }
        });
        Self { address, task }
    }

    pub(super) fn origin(&self) -> String {
        format!("https://localhost:{}", self.address.port())
    }
}

pub(super) async fn publish_observed_source(
    router: &Router,
    cookie: &str,
) -> (
    tempfile::TempDir,
    service::AppState,
    String,
    String,
    db::sync_transport::WriterRegistration,
) {
    publish_observed_source_with_trash(router, cookie, true).await
}

async fn publish_observed_source_with_trash(
    router: &Router,
    cookie: &str,
    trashed: bool,
) -> (
    tempfile::TempDir,
    service::AppState,
    String,
    String,
    db::sync_transport::WriterRegistration,
) {
    const HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let (dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(router, cookie, &source).await;
    db::sync_peer::bind_peer(&source.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    let global = db::random_hex_32();
    let group = sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('edited','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(&global).execute(&*source.db).await.unwrap().last_insert_rowid();
    let path = dir.path().join("logical.stl");
    std::fs::write(&path, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &source.db,
        &HASH[..32],
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
    if trashed {
        db::model_db::trash_models(&source.db, &user, &[model])
            .await
            .unwrap();
    }
    let deleted: Option<String> =
        sqlx::query_scalar("SELECT model_deleted FROM models WHERE model_id=?")
            .bind(model)
            .fetch_one(&*source.db)
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
    let body = serde_json::json!({"protocol":1,"connection_id":proof.connection_id,"writer_nonce":proof.writer_nonce,"item":prepared.item()});
    let boundary = "test-pull-observed-boundary";
    let bytes = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"request\"\r\nContent-Type: application/json\r\n\r\n{body}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"blob.{HASH}\"; filename=\"data\"\r\nContent-Type: application/octet-stream\r\n\r\nabc\r\n--{boundary}--\r\n"
    );
    let request = Request::post("/api/v2/sync/observations")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .header(header::COOKIE, cookie)
        .header("x-sync-writer-secret", &writer.writer_secret)
        .body(Body::from(bytes))
        .unwrap();
    let result = router.clone().oneshot(request).await.unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    drop(prepared);
    (dir, source, global, deleted.unwrap_or_default(), writer)
}

#[tokio::test]
async fn connection_requires_https_trusted_certificate_and_matching_hostname() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(&receiver.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    db::user_db::set_user_sync_token(
        &hub.db,
        db::user_db::get_user_by_email(&hub.db, "alice@example.com")
            .await
            .unwrap()
            .unwrap()
            .id,
        &token,
        &server.origin(),
        false,
    )
    .await
    .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &writer)
        .await
        .unwrap();

    let trust = || HubTrust::AdditionalCaPem(TEST_CA.to_vec());
    let connect = |origin: String, trust| {
        let (receiver, user, identity, token) = (&receiver, &user, &identity, &token);
        async move { AuthenticatedHub::connect(receiver, user, identity, &origin, token, trust).await }
    };
    assert!(
        connect(
            format!("http://localhost:{}", server.address.port()),
            trust()
        )
        .await
        .is_err()
    );
    assert!(connect(server.origin(), HubTrust::System).await.is_err());
    let wrong_hostname = format!("https://127.0.0.1:{}", server.address.port());
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &wrong_hostname, false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &writer)
        .await
        .unwrap();
    assert!(connect(wrong_hostname, trust()).await.is_err());
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &writer)
        .await
        .unwrap();
    assert!(connect(server.origin(), trust()).await.is_ok());
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Keep install and restart assertions in one end-to-end scenario.
async fn third_device_installs_observation_once_from_authenticated_hub() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, _source, global, deleted, _) =
        publish_observed_source(&router, &cookie).await;
    let (receiver_dir, receiver) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(&receiver.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    db::user_db::set_user_sync_token(
        &hub.db,
        db::user_db::get_user_by_email(&hub.db, "alice@example.com")
            .await
            .unwrap()
            .unwrap()
            .id,
        &token,
        &server.origin(),
        false,
    )
    .await
    .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &writer)
        .await
        .unwrap();
    let origin = server.origin();
    let connect = || {
        AuthenticatedHub::connect(
            &receiver,
            &user,
            &identity,
            &origin,
            &token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
        )
    };
    let client = connect()
        .await
        .expect("TLS login and account identity should authenticate");
    assert_eq!(
        client.pull_next(&receiver, &user, &identity).await.unwrap(),
        PullOutcome::Applied {
            sequence: "1".into()
        }
    );
    let installed: (String, String, Option<String>, String, String) = sqlx::query_as("SELECT g.group_unique_global_id,m.model_name,m.model_deleted,r.resource_name,l.label_name FROM models_group g JOIN models m ON m.model_group_id=g.group_id JOIN resources r ON r.resource_id=g.group_resource_id JOIN models_labels ml ON ml.model_id=m.model_id JOIN labels l ON l.label_id=ml.label_id WHERE g.group_user_id=? AND g.group_unique_global_id=?")
        .bind(user.id).bind(&global).fetch_one(&*receiver.db).await.unwrap();
    assert_eq!(
        installed,
        (
            global,
            "part".into(),
            Some(deleted),
            "project".into(),
            "label".into()
        )
    );
    let blob = db::blob_db::get_blob_via_sha256(&receiver.db, "ba7816bf8f01cfea414140de5dae2223")
        .await
        .unwrap()
        .unwrap();
    let installed_path = service::export_service::get_model_path_for_blob(&blob, &receiver);
    assert_eq!(std::fs::read(&installed_path).unwrap(), b"abc");
    assert!(
        installed_path.starts_with(receiver.get_model_dir()),
        "download must create receiver-owned storage"
    );
    let before: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM blobs)").fetch_one(&*receiver.db).await.unwrap();
    drop(client);
    receiver.db.close().await;
    drop(receiver);
    let data_path = receiver_dir.path().to_string_lossy().to_string();
    let db = db::db_context::setup_db(
        &receiver_dir.path().join("db.sqlite"),
        &receiver_dir.path().join("backups"),
    )
    .await;
    let receiver = service::AppState {
        db: Arc::new(db),
        configuration: std::sync::Mutex::new(service::Configuration {
            data_path: data_path.clone(),
            ..Default::default()
        }),
        import_mutex: Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: data_path,
    };
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &identity,
        &origin,
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert!(matches!(
        client.pull_next(&receiver, &user, &identity).await.unwrap(),
        PullOutcome::Idle { .. }
    ));
    let after: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM blobs)").fetch_one(&*receiver.db).await.unwrap();
    assert_eq!(
        before, after,
        "reauthenticated retry must not duplicate data"
    );
}

#[tokio::test]
async fn identity_redirect_or_forgery_never_receives_device_secret() {
    for redirect in [false, true] {
        let (_hub_dir, hub, router, cookie) = app().await;
        let (_receiver_dir, receiver) = crate::test_support::app_state().await;
        let user = db::model::user::User::default();
        let identity = db::user_db::get_sync_identity(&receiver.db, user.id)
            .await
            .unwrap()
            .unwrap();
        let writer = registration(&router, &cookie, &receiver).await;
        db::sync_peer::bind_peer(&receiver.db, &user, &identity, &writer.server_identity)
            .await
            .unwrap();
        let seen_secret = Arc::new(AtomicBool::new(false));
        let seen_identity = Arc::new(AtomicBool::new(false));
        let router = router.layer(middleware::from_fn_with_state((seen_secret.clone(), seen_identity.clone(), redirect),
            |State((seen, seen_identity, redirect)): State<(Arc<AtomicBool>, Arc<AtomicBool>, bool)>, request: Request<Body>, next: Next| async move {
                if request.headers().contains_key("x-sync-writer-secret") { seen.store(true, Ordering::SeqCst); }
                if request.uri().path() == "/api/v2/sync/identity" {
                    seen_identity.store(true, Ordering::SeqCst);
                    if redirect { return (StatusCode::TEMPORARY_REDIRECT, [(header::LOCATION, "https://attacker.invalid/")]).into_response(); }
                    return axum::Json(serde_json::json!({"protocol":1,"server_identity":{"instance_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","account_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}})).into_response();
                }
                next.run(request).await
            }));
        let server = TlsHub::serve(router).await;
        let token = db::random_hex_32();
        db::user_db::set_user_sync_token(
            &hub.db,
            db::user_db::get_user_by_email(&hub.db, "alice@example.com")
                .await
                .unwrap()
                .unwrap()
                .id,
            &token,
            &server.origin(),
            false,
        )
        .await
        .unwrap();
        db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
            .await
            .unwrap();
        db::sync_transport::install_authority(&receiver.db, &user, &identity, &writer)
            .await
            .unwrap();
        assert!(
            AuthenticatedHub::connect(
                &receiver,
                &user,
                &identity,
                &server.origin(),
                &token,
                HubTrust::AdditionalCaPem(TEST_CA.to_vec())
            )
            .await
            .is_err()
        );
        assert!(
            seen_identity.load(Ordering::SeqCst),
            "the forged identity response must actually be reached"
        );
        assert!(
            !seen_secret.load(Ordering::SeqCst),
            "device secret must not be sent before identity authentication"
        );
    }
}

#[tokio::test]
async fn occupied_receiver_identity_cannot_install_or_advance_cursor() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, _source, global, _, _) = publish_observed_source(&router, &cookie).await;
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(&receiver.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    sqlx::query("INSERT INTO models_group(group_name,group_created,group_user_id,group_last_modified,group_unique_global_id) VALUES('existing','2020-01-01T00:00:00Z',?,'2020-01-01T00:00:00Z',?)").bind(user.id).bind(&global).execute(&*receiver.db).await.unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    db::user_db::set_user_sync_token(
        &hub.db,
        db::user_db::get_user_by_email(&hub.db, "alice@example.com")
            .await
            .unwrap()
            .unwrap()
            .id,
        &token,
        &server.origin(),
        false,
    )
    .await
    .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert!(client.pull_next(&receiver, &user, &identity).await.is_err());
    let name: String = sqlx::query_scalar(
        "SELECT group_name FROM models_group WHERE group_unique_global_id=? AND group_user_id=?",
    )
    .bind(global)
    .bind(user.id)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    assert_eq!(name, "existing");
    let models: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models WHERE model_user_id=?")
        .bind(user.id)
        .fetch_one(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(models, 0);
    let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_pull_receipts")
        .fetch_one(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(receipts, 0, "a failed install cannot acknowledge a page");
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Both failures and retry must share the same delivery.
async fn corrupted_download_and_receipt_failure_leave_no_install_or_cursor() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, _source, global, _, _) = publish_observed_source(&router, &cookie).await;
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(&receiver.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    let corrupt = Arc::new(AtomicBool::new(true));
    let bad = corrupt.clone();
    let router = router.layer(middleware::from_fn(
        move |request: Request<Body>, next: Next| {
            let bad = bad.clone();
            async move {
                if request.uri().path().contains("/blobs/") && bad.load(Ordering::SeqCst) {
                    return (StatusCode::OK, "abd").into_response();
                }
                next.run(request).await
            }
        },
    ));
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    db::user_db::set_user_sync_token(
        &hub.db,
        db::user_db::get_user_by_email(&hub.db, "alice@example.com")
            .await
            .unwrap()
            .unwrap()
            .id,
        &token,
        &server.origin(),
        false,
    )
    .await
    .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let model_dir = receiver.get_model_dir();
    let entries = || {
        std::fs::read_dir(&model_dir)
            .unwrap()
            .filter(|entry| {
                let path = entry.as_ref().unwrap().path();
                !(path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".sync-stage-")
                    && path.is_dir()
                    && std::fs::read_dir(&path).unwrap().all(|entry| {
                        let entry = entry.unwrap();
                        entry.file_name() == ".lease" && entry.file_type().unwrap().is_file()
                    }))
            })
            .count()
    };
    assert_eq!(entries(), 0);
    assert!(
        client.pull_next(&receiver, &user, &identity).await.is_err(),
        "a corrupted logical blob must be rejected"
    );
    let count = || async {
        sqlx::query_as::<_,(i64,i64,i64)>("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM sync_pull_receipts)").fetch_one(&*receiver.db).await.unwrap()
    };
    assert_eq!(count().await, (0, 0, 0));
    assert_eq!(entries(), 0, "corrupt download must remove staging files");
    corrupt.store(false, Ordering::SeqCst);
    sqlx::query("CREATE TRIGGER fail_test_pull BEFORE INSERT ON sync_pull_receipts BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END").execute(&*receiver.db).await.unwrap();
    assert!(client.pull_next(&receiver, &user, &identity).await.is_err());
    assert_eq!(
        count().await,
        (0, 0, 0),
        "receipt failure must roll back the entire install"
    );
    assert_eq!(
        entries(),
        0,
        "receipt failure must remove uncommitted files"
    );
    sqlx::query("DROP TRIGGER fail_test_pull")
        .execute(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(
        client.pull_next(&receiver, &user, &identity).await.unwrap(),
        PullOutcome::Applied {
            sequence: "1".into()
        }
    );
    let installed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM models_group WHERE group_unique_global_id=?")
            .bind(global)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    assert_eq!(installed, 1, "retry after failures must install once");
}

#[tokio::test]
async fn revoked_and_restored_link_cannot_commit_an_inflight_download() {
    Box::pin(assert_inflight_revocation(0)).await;
}

#[tokio::test]
async fn replacement_local_account_cannot_receive_inflight_download() {
    Box::pin(assert_inflight_revocation(1)).await;
}

#[tokio::test]
async fn replacement_hub_account_cannot_serve_inflight_download() {
    Box::pin(assert_inflight_revocation(2)).await;
}

#[tokio::test]
async fn concurrent_identical_delivery_is_applied_once_and_acknowledged_twice() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, _source, _global, _, _) = publish_observed_source(&router, &cookie).await;
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(&receiver.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let router = router.layer(middleware::from_fn(
        move |request: Request<Body>, next: Next| {
            let barrier = barrier.clone();
            async move {
                if request.uri().path().contains("/blobs/") {
                    barrier.wait().await;
                }
                next.run(request).await
            }
        },
    ));
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let (first, second) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(
            client.pull_next(&receiver, &user, &identity),
            client.pull_next(&receiver, &user, &identity)
        )
    })
    .await
    .expect("both pulls must reach the same delivery");
    let results = [first.unwrap(), second.unwrap()];
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, PullOutcome::Applied { .. }))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, PullOutcome::AlreadyApplied { .. }))
            .count(),
        1
    );
    let counts: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM sync_pull_receipts)").fetch_one(&*receiver.db).await.unwrap();
    assert_eq!(counts, (1, 1, 1));
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // The same authenticated page is tampered in several independent ways before a valid retry.
async fn malformed_page_cannot_install_or_advance_cursor() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, _source, _, _, _) = publish_observed_source(&router, &cookie).await;
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(&receiver.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    let attack = Arc::new(AtomicUsize::new(1));
    let mode = attack.clone();
    let router = router.layer(middleware::from_fn(
        move |request: Request<Body>, next: Next| {
            let mode = mode.clone();
            async move {
                let page = request.uri().path() == "/api/v2/sync/items";
                let response = next.run(request).await;
                let mode = mode.load(Ordering::SeqCst);
                if !page || mode == 0 {
                    return response;
                }
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
                    .await
                    .unwrap();
                let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                match mode {
                    1 => json["items"][0]["sequence"] = "2".into(),
                    2 => json["items"][0]["digest"] = "0".repeat(64).into(),
                    3 => json["items"][0]["issuer"]["writer_id"] = "f".repeat(32).into(),
                    4 => json["unexpected"] = true.into(),
                    _ => unreachable!(),
                }
                axum::Json(json).into_response()
            }
        },
    ));
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    for mode in 1..=4 {
        attack.store(mode, Ordering::SeqCst);
        assert!(
            client.pull_next(&receiver, &user, &identity).await.is_err(),
            "tampered page mode {mode} must be rejected"
        );
        let counts: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM sync_pull_receipts)").fetch_one(&*receiver.db).await.unwrap();
        assert_eq!(
            counts,
            (0, 0, 0),
            "tampered page mode {mode} cannot advance or install"
        );
    }
    attack.store(0, Ordering::SeqCst);
    assert_eq!(
        client.pull_next(&receiver, &user, &identity).await.unwrap(),
        PullOutcome::Applied {
            sequence: "1".into()
        }
    );
}

#[tokio::test]
async fn source_echo_acknowledges_exact_observation_without_overwriting_later_edits() {
    for later_edit in [false, true] {
        let (_hub_dir, hub, router, cookie) = app().await;
        let (_source_dir, source, global, _, writer) =
            publish_observed_source(&router, &cookie).await;
        let user = db::model::user::User::default();
        let identity = db::user_db::get_sync_identity(&source.db, user.id)
            .await
            .unwrap()
            .unwrap();
        let group: i64 = sqlx::query_scalar(
            "SELECT group_id FROM models_group WHERE group_unique_global_id=? AND group_user_id=?",
        )
        .bind(&global)
        .bind(user.id)
        .fetch_one(&*source.db)
        .await
        .unwrap();
        let (router, forge) = forge_page_issuer(router);
        let server = TlsHub::serve(router).await;
        let token = db::random_hex_32();
        let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
            .await
            .unwrap()
            .unwrap();
        db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
            .await
            .unwrap();
        db::user_db::set_user_sync_token(&source.db, user.id, &token, &server.origin(), false)
            .await
            .unwrap();
        db::sync_transport::install_authority(&source.db, &user, &identity, &writer)
            .await
            .unwrap();
        if later_edit {
            db::group_db::edit_group(&source.db, &user, group, "later local edit", None, None)
                .await
                .unwrap();
        }
        let before: (String,i64,i64) = sqlx::query_as("SELECT (SELECT group_name FROM models_group WHERE group_id=?),(SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM blobs)").bind(group).fetch_one(&*source.db).await.unwrap();
        let client = AuthenticatedHub::connect(
            &source,
            &user,
            &identity,
            &server.origin(),
            &token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
        )
        .await
        .unwrap();
        assert!(
            client.pull_next(&source, &user, &identity).await.is_err(),
            "another writer cannot claim the source's own observation"
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*source.db)
            .await
            .unwrap();
        assert_eq!(count, 0);
        forge.store(false, Ordering::SeqCst);
        assert!(
            matches!(client.pull_next(&source, &user, &identity).await.unwrap(), PullOutcome::AlreadyApplied { ref sequence } if sequence == "1")
        );
        let after: (String,i64,i64) = sqlx::query_as("SELECT (SELECT group_name FROM models_group WHERE group_id=?),(SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM blobs)").bind(group).fetch_one(&*source.db).await.unwrap();
        assert_eq!(
            before, after,
            "source echo must never reinstall or overwrite later changes"
        );
        let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*source.db)
            .await
            .unwrap();
        assert_eq!(receipts, 1);
    }
}

fn forge_page_issuer(router: Router) -> (Router, Arc<AtomicBool>) {
    let forge = Arc::new(AtomicBool::new(true));
    let forge_page = forge.clone();
    let router = router.layer(middleware::from_fn(
        move |request: Request<Body>, next: Next| {
            let forge = forge_page.clone();
            async move {
                let page = request.uri().path() == "/api/v2/sync/items";
                let response = next.run(request).await;
                if !page || !forge.load(Ordering::SeqCst) {
                    return response;
                }
                let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
                    .await
                    .unwrap();
                let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                json["items"][0]["issuer"]["writer_id"] = "f".repeat(32).into();
                axum::Json(json).into_response()
            }
        },
    ));
    (router, forge)
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One stream must preserve ordering across distinct delivery kinds and an injected failure.
async fn shared_stream_installs_genesis_operation_and_observation_in_order() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_author_dir, author) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let author_identity = db::user_db::get_sync_identity(&author.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let author_writer = registration(&router, &cookie, &author).await;
    db::sync_peer::bind_peer(
        &author.db,
        &user,
        &author_identity,
        &author_writer.server_identity,
    )
    .await
    .unwrap();
    db::sync_transport::activate_writer(&author.db, &user, &author_identity, &author_writer)
        .await
        .unwrap();
    let group = db::group_db::add_empty_group(&author.db, &user, "initial", None)
        .await
        .unwrap();
    let root = db::causal_group_lifecycle::enroll_pristine_group(
        &author.db,
        &user,
        &author_identity,
        group.id,
    )
    .await
    .unwrap();
    let genesis = db::sync_transport::encode_genesis(&root).unwrap();
    let result = router
        .clone()
        .oneshot(publish(
            &author_writer,
            &genesis,
            &cookie,
            &author_writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    db::group_db::edit_group(&author.db, &user, group.id, "renamed", None, None)
        .await
        .unwrap();
    let state =
        db::causal_group_lifecycle::get_state(&author.db, &user, &author_identity, group.id)
            .await
            .unwrap();
    let NameState::Ready { token, .. } = state.name else {
        panic!("expected completed rename")
    };
    let operation = db::causal_group_lifecycle::export_operation(
        &author.db,
        &user,
        &author_identity,
        &token.frontier[0],
    )
    .await
    .unwrap();
    let item = db::sync_transport::encode_operation(&operation).unwrap();
    let result = router
        .clone()
        .oneshot(publish(
            &author_writer,
            &item,
            &cookie,
            &author_writer.writer_secret,
        ))
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let (_observation_dir, _observation, populated_global, _, _) =
        publish_observed_source(&router, &cookie).await;
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let identity = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(&receiver.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert_eq!(
        client.pull_next(&receiver, &user, &identity).await.unwrap(),
        PullOutcome::Applied {
            sequence: "1".into()
        }
    );
    let name = || async {
        sqlx::query_scalar::<_,String>("SELECT group_name FROM models_group WHERE group_user_id=? AND group_unique_global_id=?").bind(user.id).bind(&root.entity().global_id).fetch_one(&*receiver.db).await.unwrap()
    };
    assert_eq!(name().await, "initial");
    sqlx::query("CREATE TRIGGER fail_operation_receipt BEFORE INSERT ON sync_pull_receipts WHEN NEW.sequence=2 BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END").execute(&*receiver.db).await.unwrap();
    assert!(client.pull_next(&receiver, &user, &identity).await.is_err());
    assert_eq!(
        name().await,
        "initial",
        "failed receipt must roll back the rename projection"
    );
    let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_pull_receipts")
        .fetch_one(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(receipts, 1);
    sqlx::query("DROP TRIGGER fail_operation_receipt")
        .execute(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(
        client.pull_next(&receiver, &user, &identity).await.unwrap(),
        PullOutcome::Applied {
            sequence: "2".into()
        }
    );
    assert_eq!(name().await, "renamed");
    assert_eq!(
        client.pull_next(&receiver, &user, &identity).await.unwrap(),
        PullOutcome::Applied {
            sequence: "3".into()
        }
    );
    let populated: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(user.id)
    .bind(populated_global)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    assert_eq!(populated, 1);
    assert!(matches!(
        client.pull_next(&receiver, &user, &identity).await.unwrap(),
        PullOutcome::Idle { .. }
    ));
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Both source-authored items must acknowledge without projecting over a later local change.
async fn source_echo_of_genesis_and_operation_preserves_later_local_edit() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
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
    let group = db::group_db::add_empty_group(&source.db, &user, "initial", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    let proof = db::sync_transport::writer_proof(&source.db, &user, &identity, root.event())
        .await
        .unwrap();
    let genesis = serde_json::json!({"protocol":1,"connection_id":proof.connection_id,"writer_nonce":proof.writer_nonce,"item":db::sync_transport::encode_genesis(&root).unwrap()});
    let response = router
        .clone()
        .oneshot(post(
            "/api/v2/sync/items",
            &genesis,
            Some(&cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    db::group_db::edit_group(&source.db, &user, group.id, "published rename", None, None)
        .await
        .unwrap();
    let state = db::causal_group_lifecycle::get_state(&source.db, &user, &identity, group.id)
        .await
        .unwrap();
    let NameState::Ready { token, .. } = state.name else {
        panic!("expected completed rename")
    };
    let dot = &token.frontier[0];
    let proof = db::sync_transport::writer_proof(&source.db, &user, &identity, dot)
        .await
        .unwrap();
    let op = db::causal_group_lifecycle::export_operation(&source.db, &user, &identity, dot)
        .await
        .unwrap();
    let item = serde_json::json!({"protocol":1,"connection_id":proof.connection_id,"writer_nonce":proof.writer_nonce,"item":db::sync_transport::encode_operation(&op).unwrap()});
    let response = router
        .clone()
        .oneshot(post(
            "/api/v2/sync/items",
            &item,
            Some(&cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(&source.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    db::group_db::edit_group(&source.db, &user, group.id, "later local edit", None, None)
        .await
        .unwrap();
    let before: (String,i64,i64) = sqlx::query_as("SELECT (SELECT group_name FROM models_group WHERE group_id=?),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM causal_events)").bind(group.id).fetch_one(&*source.db).await.unwrap();
    let client = AuthenticatedHub::connect(
        &source,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert_eq!(
        client.pull_next(&source, &user, &identity).await.unwrap(),
        PullOutcome::AlreadyApplied {
            sequence: "1".into()
        }
    );
    assert_eq!(
        client.pull_next(&source, &user, &identity).await.unwrap(),
        PullOutcome::AlreadyApplied {
            sequence: "2".into()
        }
    );
    let after: (String,i64,i64) = sqlx::query_as("SELECT (SELECT group_name FROM models_group WHERE group_id=?),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM causal_events)").bind(group.id).fetch_one(&*source.db).await.unwrap();
    assert_eq!(before, after);
    assert_eq!(after.0, "later local edit");
    let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_pull_receipts")
        .fetch_one(&*source.db)
        .await
        .unwrap();
    assert_eq!(receipts, 2);
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // The receiving device lacks a model root until an explicit dependency installation.
async fn missing_model_root_blocks_the_stream_without_acknowledging_the_move() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &source).await;
    db::sync_peer::bind_peer(&source.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    db::sync_transport::activate_writer(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    let group = db::group_db::add_empty_group(&source.db, &user, "destination", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    let genesis = db::sync_transport::encode_genesis(&root).unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(publish(&writer, &genesis, &cookie, &writer.writer_secret))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let hash = db::random_hex_32();
    let source_blob = db::blob_db::add_blob(&source.db, &hash, "stl", 1, None)
        .await
        .unwrap();
    let model = db::model_db::add_model(&source.db, &user, "part", source_blob, None, None)
        .await
        .unwrap();
    let model_root =
        db::causal_group_lifecycle::enroll_pristine_model(&source.db, &user, &identity, model)
            .await
            .unwrap();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    let hub_identity = db::user_db::get_sync_identity(&hub.db, hub_user.id)
        .await
        .unwrap()
        .unwrap();
    let peer = db::sync_peer::bind_peer(&hub.db, &hub_user, &hub_identity, &identity)
        .await
        .unwrap();
    let hub_blob = db::blob_db::add_blob(&hub.db, &hash, "stl", 1, None)
        .await
        .unwrap();
    db::causal_genesis::install_pristine_genesis(
        &hub.db,
        &hub_user,
        &hub_identity,
        &peer,
        &model_root,
        Some(hub_blob),
    )
    .await
    .unwrap();
    db::group_db::set_group_id_on_models(&source.db, &user, Some(group.id), vec![model], None)
        .await
        .unwrap();
    let MoveState::Ready { token, .. } =
        db::causal_model_move::get_state(&source.db, &user, &identity, model)
            .await
            .unwrap()
    else {
        panic!("expected move")
    };
    let op = db::causal_group_lifecycle::export_operation(
        &source.db,
        &user,
        &identity,
        &token.frontier[0],
    )
    .await
    .unwrap();
    let item = db::sync_transport::encode_operation(&op).unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let local = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let receiver_writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(
        &receiver.db,
        &user,
        &local,
        &receiver_writer.server_identity,
    )
    .await
    .unwrap();
    let server = TlsHub::serve(router).await;
    let login_token = db::random_hex_32();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &login_token, &server.origin(), false)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &login_token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &local, &receiver_writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &local,
        &server.origin(),
        &login_token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert_eq!(
        client.pull_next(&receiver, &user, &local).await.unwrap(),
        PullOutcome::Applied {
            sequence: "1".into()
        }
    );
    let error = client
        .pull_next(&receiver, &user, &local)
        .await
        .unwrap_err();
    assert!(
        format!("{error:?}").contains("needs_dependencies"),
        "missing model root should be explicit: {error:?}"
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_pull_receipts")
        .fetch_one(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let peer = db::sync_peer::bind_peer(&receiver.db, &user, &local, &identity)
        .await
        .unwrap();
    let blob = db::blob_db::add_blob(&receiver.db, &hash, "stl", 1, None)
        .await
        .unwrap();
    db::causal_genesis::install_pristine_genesis(
        &receiver.db,
        &user,
        &local,
        &peer,
        &model_root,
        Some(blob),
    )
    .await
    .unwrap();
    assert_eq!(
        client.pull_next(&receiver, &user, &local).await.unwrap(),
        PullOutcome::Applied {
            sequence: "2".into()
        }
    );
    let destination: Option<String> = sqlx::query_scalar("SELECT g.group_unique_global_id FROM models m JOIN models_group g ON g.group_id=m.model_group_id WHERE m.model_user_id=? AND m.model_name='part'").bind(user.id).fetch_optional(&*receiver.db).await.unwrap();
    assert_eq!(
        destination.as_deref(),
        Some(root.entity().global_id.as_str())
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // The receiver must project deletion and restoration from distinct ordered receipts.
async fn lifecycle_delete_and_restore_apply_over_the_ordered_https_stream() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, source) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &source).await;
    db::sync_peer::bind_peer(&source.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    db::sync_transport::activate_writer(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    let group = db::group_db::add_empty_group(&source.db, &user, "folder", None)
        .await
        .unwrap();
    let root =
        db::causal_group_lifecycle::enroll_pristine_group(&source.db, &user, &identity, group.id)
            .await
            .unwrap();
    let genesis = db::sync_transport::encode_genesis(&root).unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(publish(&writer, &genesis, &cookie, &writer.writer_secret))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    db::group_db::delete_group(&source.db, &user, group.id)
        .await
        .unwrap();
    let db::causal_group_lifecycle::GroupLifecycle::Deleted { deletions } =
        db::causal_group_lifecycle::get_state(&source.db, &user, &identity, group.id)
            .await
            .unwrap()
            .lifecycle
    else {
        panic!("expected deleted group")
    };
    let delete =
        db::causal_group_lifecycle::export_operation(&source.db, &user, &identity, &deletions[0])
            .await
            .unwrap();
    let item = db::sync_transport::encode_operation(&delete).unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let deleted = db::group_db::get_trashed_groups(&source.db, &user, 30)
        .await
        .unwrap()
        .into_iter()
        .find(|entry| entry.id == group.id)
        .unwrap();
    let request = db::group_db::GroupRestoreRequest {
        id: deleted.id,
        group_global_id: deleted.group_global_id,
        deletion_operation_id: deleted.deletion_operation_id.unwrap(),
    };
    db::group_db::restore_groups(&source.db, &user, &[request], 30)
        .await
        .unwrap();
    let NameState::Ready { token, .. } =
        db::causal_group_lifecycle::get_state(&source.db, &user, &identity, group.id)
            .await
            .unwrap()
            .name
    else {
        panic!("expected restored group")
    };
    let restore = db::causal_group_lifecycle::export_operation(
        &source.db,
        &user,
        &identity,
        &token.frontier[0],
    )
    .await
    .unwrap();
    let item = db::sync_transport::encode_operation(&restore).unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(publish(&writer, &item, &cookie, &writer.writer_secret))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let local = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let receiver_writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(
        &receiver.db,
        &user,
        &local,
        &receiver_writer.server_identity,
    )
    .await
    .unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &local, &receiver_writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &local,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert_eq!(
        client.pull_next(&receiver, &user, &local).await.unwrap(),
        PullOutcome::Applied {
            sequence: "1".into()
        }
    );
    assert_eq!(
        client.pull_next(&receiver, &user, &local).await.unwrap(),
        PullOutcome::Applied {
            sequence: "2".into()
        }
    );
    let trash = db::group_db::get_trashed_groups(&receiver.db, &user, 30)
        .await
        .unwrap();
    assert!(
        trash
            .iter()
            .any(|entry| entry.group_global_id == root.entity().global_id)
    );
    assert_eq!(
        client.pull_next(&receiver, &user, &local).await.unwrap(),
        PullOutcome::Applied {
            sequence: "3".into()
        }
    );
    let trash = db::group_db::get_trashed_groups(&receiver.db, &user, 30)
        .await
        .unwrap();
    assert!(
        trash
            .iter()
            .all(|entry| entry.group_global_id != root.entity().global_id)
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Two independently authored renames must survive delivery as user-selectable choices.
async fn concurrent_rename_choices_survive_ordered_https_pull() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_a_dir, a) = crate::test_support::app_state().await;
    let (_b_dir, b) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let ai = db::user_db::get_sync_identity(&a.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let bi = db::user_db::get_sync_identity(&b.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let aw = registration(&router, &cookie, &a).await;
    let bw = registration(&router, &cookie, &b).await;
    for (device, identity, writer) in [(&a, &ai, &aw), (&b, &bi, &bw)] {
        db::sync_peer::bind_peer(&device.db, &user, identity, &writer.server_identity)
            .await
            .unwrap();
        db::sync_transport::activate_writer(&device.db, &user, identity, writer)
            .await
            .unwrap();
    }
    let group = db::group_db::add_empty_group(&a.db, &user, "original", None)
        .await
        .unwrap();
    let root = db::causal_group_lifecycle::enroll_pristine_group(&a.db, &user, &ai, group.id)
        .await
        .unwrap();
    let genesis = db::sync_transport::encode_genesis(&root).unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(publish(&aw, &genesis, &cookie, &aw.writer_secret))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let peer = db::sync_peer::bind_peer(&b.db, &user, &bi, &ai)
        .await
        .unwrap();
    let b_group =
        db::causal_genesis::install_pristine_genesis(&b.db, &user, &bi, &peer, &root, None)
            .await
            .unwrap()
            .local_id;
    for (device, identity, id, name, writer) in [
        (&a, &ai, group.id, "alpha", &aw),
        (&b, &bi, b_group, "bravo", &bw),
    ] {
        db::group_db::edit_group(&device.db, &user, id, name, None, None)
            .await
            .unwrap();
        let state = db::causal_group_lifecycle::get_state(&device.db, &user, identity, id)
            .await
            .unwrap();
        let NameState::Ready { token, .. } = state.name else {
            panic!("expected source rename")
        };
        let op = db::causal_group_lifecycle::export_operation(
            &device.db,
            &user,
            identity,
            &token.frontier[0],
        )
        .await
        .unwrap();
        let item = db::sync_transport::encode_operation(&op).unwrap();
        assert_eq!(
            router
                .clone()
                .oneshot(publish(writer, &item, &cookie, &writer.writer_secret))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let local = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(&receiver.db, &user, &local, &writer.server_identity)
        .await
        .unwrap();
    let server = TlsHub::serve(router).await;
    let login_token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &login_token, &server.origin(), false)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &login_token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &local, &writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &local,
        &server.origin(),
        &login_token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    for sequence in 1..=3 {
        assert_eq!(
            client.pull_next(&receiver, &user, &local).await.unwrap(),
            PullOutcome::Applied {
                sequence: sequence.to_string()
            }
        );
    }
    let local_group: i64 = sqlx::query_scalar(
        "SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(user.id)
    .bind(&root.entity().global_id)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    let NameState::Conflict { choices, token } =
        db::causal_group_lifecycle::get_state(&receiver.db, &user, &local, local_group)
            .await
            .unwrap()
            .name
    else {
        panic!("both independent names must be offered")
    };
    let mut names: Vec<_> = choices.iter().map(|choice| choice.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["alpha", "bravo"]);
    let resolved =
        db::causal_group_rename::resolve(&receiver.db, &user, &local, local_group, &token, "alpha")
            .await
            .unwrap();
    assert!(
        matches!(resolved, ResolveResult::Resolved(NameState::Ready { ref name, .. }) if name == "alpha")
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Independent authenticated stream must acknowledge exact existing proof without overwriting later edits.
async fn installed_observation_on_new_stream_keeps_later_local_edits() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, _source, global, _, _) = publish_observed_source(&router, &cookie).await;
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(&receiver.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    let first = TlsHub::serve(router.clone()).await;
    let (router, attack) = forge_observation_page(router);
    let second = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &first.origin(), false)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &first.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &identity,
        &first.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert_eq!(
        client.pull_next(&receiver, &user, &identity).await.unwrap(),
        PullOutcome::Applied {
            sequence: "1".into()
        }
    );
    let group: i64 = sqlx::query_scalar(
        "SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    db::group_db::edit_group(&receiver.db, &user, group, "later local edit", None, None)
        .await
        .unwrap();
    let before: (String,i64,i64,i64) = sqlx::query_as("SELECT (SELECT group_name FROM models_group WHERE group_id=?),(SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM blobs),(SELECT COUNT(*) FROM causal_events)").bind(group).fetch_one(&*receiver.db).await.unwrap();
    assert_eq!(before.0, "later local edit");
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &second.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &identity,
        &second.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    for mode in [1, 2, 3, 4, 8] {
        attack.store(mode, Ordering::SeqCst);
        assert!(
            client.pull_next(&receiver, &user, &identity).await.is_err(),
            "changed full hash or issuer must not reuse the prior proof"
        );
        let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
        assert_eq!(receipts, 1);
    }
    attack.store(0, Ordering::SeqCst);
    for (select, update) in [
        (
            "SELECT server_instance_id FROM sync_pull_streams WHERE origin=?",
            "UPDATE sync_pull_streams SET server_instance_id=? WHERE origin=?",
        ),
        (
            "SELECT server_account_id FROM sync_pull_streams WHERE origin=?",
            "UPDATE sync_pull_streams SET server_account_id=? WHERE origin=?",
        ),
    ] {
        let original: String = sqlx::query_scalar(select)
            .bind(first.origin())
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
        sqlx::query(update)
            .bind(db::random_hex_32())
            .bind(first.origin())
            .execute(&*receiver.db)
            .await
            .unwrap();
        attack.store(6, Ordering::SeqCst);
        assert!(
            client.pull_next(&receiver, &user, &identity).await.is_err(),
            "a receipt from a different server identity cannot authorize a metadata-only acknowledgment"
        );
        assert_eq!(
            attack.load(Ordering::SeqCst),
            7,
            "the client must attempt to fetch missing bytes instead of accepting the unrelated receipt"
        );
        attack.store(0, Ordering::SeqCst);
        let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
        assert_eq!(receipts, 1);
        let cursor: i64 =
            sqlx::query_scalar("SELECT last_applied FROM sync_pull_streams WHERE origin=?")
                .bind(second.origin())
                .fetch_one(&*receiver.db)
                .await
                .unwrap();
        assert_eq!(cursor, 0, "missing blob must not advance the new stream");
        sqlx::query(update)
            .bind(original)
            .bind(first.origin())
            .execute(&*receiver.db)
            .await
            .unwrap();
    }
    // Simulate corrupted historical evidence in this isolated database: the
    // incoming authenticated item must not be acknowledged against it.
    sqlx::query("DROP TRIGGER immutable_sync_observation_content")
        .execute(&*receiver.db)
        .await
        .unwrap();
    let old_sha: String =
        sqlx::query_scalar("SELECT sha256 FROM sync_observation_contents WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    sqlx::query("UPDATE sync_observation_contents SET sha256=? WHERE user_id=?")
        .bind("f".repeat(64))
        .bind(user.id)
        .execute(&*receiver.db)
        .await
        .unwrap();
    assert!(
        client.pull_next(&receiver, &user, &identity).await.is_err(),
        "a damaged prior proof cannot acknowledge the item"
    );
    let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_pull_receipts")
        .fetch_one(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(receipts, 1);
    sqlx::query("UPDATE sync_observation_contents SET sha256=? WHERE user_id=?")
        .bind(old_sha)
        .bind(user.id)
        .execute(&*receiver.db)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER immutable_sync_observation_content BEFORE UPDATE ON sync_observation_contents WHEN NEW.user_id IS NOT OLD.user_id OR NEW.batch_id IS NOT OLD.batch_id OR NEW.hash IS NOT OLD.hash OR NEW.filetype IS NOT OLD.filetype OR NEW.size IS NOT OLD.size OR NEW.sha256 IS NOT OLD.sha256 OR NEW.blob_id IS NOT NULL BEGIN SELECT RAISE(ABORT,'verified content reference is immutable'); END").execute(&*receiver.db).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_duplicate_receipt BEFORE INSERT ON sync_pull_receipts BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END").execute(&*receiver.db).await.unwrap();
    assert!(client.pull_next(&receiver, &user, &identity).await.is_err());
    let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_pull_receipts")
        .fetch_one(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(
        receipts, 1,
        "failed second receipt cannot acknowledge the item"
    );
    sqlx::query("DROP TRIGGER fail_duplicate_receipt")
        .execute(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(
        client.pull_next(&receiver, &user, &identity).await.unwrap(),
        PullOutcome::AlreadyApplied {
            sequence: "1".into()
        }
    );
    let after: (String,i64,i64,i64) = sqlx::query_as("SELECT (SELECT group_name FROM models_group WHERE group_id=?),(SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM blobs),(SELECT COUNT(*) FROM causal_events)").bind(group).fetch_one(&*receiver.db).await.unwrap();
    assert_eq!(
        before, after,
        "repeated baseline must not overwrite or recreate local data"
    );
    let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_pull_receipts")
        .fetch_one(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(receipts, 2);
    assert!(matches!(
        client.pull_next(&receiver, &user, &identity).await.unwrap(),
        PullOutcome::Idle { .. }
    ));
}

fn forge_observation_page(router: Router) -> (Router, Arc<AtomicUsize>) {
    let attack = Arc::new(AtomicUsize::new(0));
    let state = attack.clone();
    let router = router.layer(middleware::from_fn(
        move |request: Request<Body>, next: Next| {
            let state = state.clone();
            async move {
                let mode = state.load(Ordering::SeqCst);
                if mode == 6 && request.uri().path().contains("/blobs/") {
                    state.store(7, Ordering::SeqCst);

                    return StatusCode::GONE.into_response();
                }
                let page = request.uri().path() == "/api/v2/sync/items";
                let response = next.run(request).await;
                if !page || mode == 0 || mode == 6 {
                    return response;
                }
                let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
                    .await
                    .unwrap();
                let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                if mode == 1 || mode == 5 {
                    if mode == 1 {
                        let sha = json["items"][0]["item"]["contents"][0]["sha256"]
                            .as_str()
                            .unwrap();
                        json["items"][0]["item"]["contents"][0]["sha256"] =
                            format!("{}{}", &sha[..32], "f".repeat(32)).into();
                    } else {
                        let proof = json["items"][0]["item"]["group"]["proof"].as_str().unwrap();
                        let mut proof: serde_json::Value = serde_json::from_str(proof).unwrap();
                        proof["batch_id"] = "f".repeat(32).into();
                        json["items"][0]["item"]["group"]["proof"] =
                            serde_json::to_string(&proof).unwrap().into();
                    }
                    let wire = serde_json::to_string(&json["items"][0]["item"]).unwrap();
                    let digest = sha2::Sha256::digest(wire.as_bytes()).iter().fold(
                        String::new(),
                        |mut out, byte| {
                            write!(out, "{byte:02x}").unwrap();
                            out
                        },
                    );
                    json["items"][0]["digest"] = digest.into();
                } else if mode == 8 {
                    json["items"][0]["digest"] = "0".repeat(64).into();
                } else {
                    let field = match mode {
                        2 => "connection_id",
                        3 => "authority_writer_id",
                        4 => "writer_id",
                        _ => unreachable!(),
                    };
                    json["items"][0]["issuer"][field] = "f".repeat(32).into();
                }
                axum::Json(json).into_response()
            }
        },
    ));
    (router, attack)
}

#[tokio::test]
async fn source_observation_can_be_acknowledged_on_a_second_stream() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, source, global, _, writer) = publish_observed_source(&router, &cookie).await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let first = TlsHub::serve(router.clone()).await;
    let second = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &first.origin(), false)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(&source.db, user.id, &token, &first.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    let first_client = AuthenticatedHub::connect(
        &source,
        &user,
        &identity,
        &first.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert_eq!(
        first_client
            .pull_next(&source, &user, &identity)
            .await
            .unwrap(),
        PullOutcome::AlreadyApplied {
            sequence: "1".into()
        }
    );
    let group: i64 = sqlx::query_scalar(
        "SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&*source.db)
    .await
    .unwrap();
    db::group_db::edit_group(&source.db, &user, group, "later source edit", None, None)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(&source.db, user.id, &token, &second.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&source.db, &user, &identity, &writer)
        .await
        .unwrap();
    let second_client = AuthenticatedHub::connect(
        &source,
        &user,
        &identity,
        &second.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert_eq!(
        second_client
            .pull_next(&source, &user, &identity)
            .await
            .unwrap(),
        PullOutcome::AlreadyApplied {
            sequence: "1".into()
        }
    );
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=?")
        .bind(group)
        .fetch_one(&*source.db)
        .await
        .unwrap();
    assert_eq!(name, "later source edit");
    let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_pull_receipts")
        .fetch_one(&*source.db)
        .await
        .unwrap();
    assert_eq!(receipts, 2);
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // The same expired item is replayed to an established receiver, its author and an unknown receiver.
async fn expired_hub_blob_allows_proven_replay_but_blocks_unknown_receiver() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, source, global, _, source_writer) =
        publish_observed_source(&router, &cookie).await;
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let (_unknown_dir, unknown) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let source_identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let receiver_identity = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let unknown_identity = db::user_db::get_sync_identity(&unknown.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &receiver).await;
    let unknown_writer = registration(&router, &cookie, &unknown).await;
    for (device, identity, authority) in [
        (&receiver, &receiver_identity, &writer),
        (&unknown, &unknown_identity, &unknown_writer),
    ] {
        db::sync_peer::bind_peer(&device.db, &user, identity, &authority.server_identity)
            .await
            .unwrap();
    }
    let first = TlsHub::serve(router.clone()).await;
    let second = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &first.origin(), false)
        .await
        .unwrap();
    for (device, identity, authority) in [
        (&source, &source_identity, &source_writer),
        (&receiver, &receiver_identity, &writer),
    ] {
        db::user_db::set_user_sync_token(&device.db, user.id, &token, &first.origin(), false)
            .await
            .unwrap();
        db::sync_transport::install_authority(&device.db, &user, identity, authority)
            .await
            .unwrap();
        let client = AuthenticatedHub::connect(
            device,
            &user,
            identity,
            &first.origin(),
            &token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
        )
        .await
        .unwrap();
        let result = client.pull_next(device, &user, identity).await.unwrap();
        assert!(matches!(
            result,
            PullOutcome::Applied { .. } | PullOutcome::AlreadyApplied { .. }
        ));
    }
    let target_group: i64 = sqlx::query_scalar(
        "SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    db::group_db::edit_group(
        &receiver.db,
        &user,
        target_group,
        "preserve local edit",
        None,
        None,
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
    let content: Option<i64> =
        sqlx::query_scalar("SELECT blob_id FROM sync_observation_contents WHERE user_id=?")
            .bind(hub_user.id)
            .fetch_one(&*hub.db)
            .await
            .unwrap();
    assert_eq!(
        content, None,
        "hub receipt survives after its content expires"
    );
    for (device, identity, authority) in [
        (&source, &source_identity, &source_writer),
        (&receiver, &receiver_identity, &writer),
    ] {
        db::user_db::set_user_sync_token(&device.db, user.id, &token, &second.origin(), false)
            .await
            .unwrap();
        db::sync_transport::install_authority(&device.db, &user, identity, authority)
            .await
            .unwrap();
        let client = AuthenticatedHub::connect(
            device,
            &user,
            identity,
            &second.origin(),
            &token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
        )
        .await
        .unwrap();
        assert_eq!(
            client.pull_next(device, &user, identity).await.unwrap(),
            PullOutcome::AlreadyApplied {
                sequence: "1".into()
            }
        );
    }
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=?")
        .bind(target_group)
        .fetch_one(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(name, "preserve local edit");
    db::user_db::set_user_sync_token(&unknown.db, user.id, &token, &second.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&unknown.db, &user, &unknown_identity, &unknown_writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &unknown,
        &user,
        &unknown_identity,
        &second.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert!(
        client
            .pull_next(&unknown, &user, &unknown_identity)
            .await
            .is_err(),
        "unseen baseline cannot be installed without its bytes"
    );
    let counts: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM sync_pull_receipts)").fetch_one(&*unknown.db).await.unwrap();
    assert_eq!(counts, (0, 0, 0));
}

async fn publish_group_name_operation(
    router: &Router,
    cookie: &str,
    source: &service::AppState,
    user: &db::model::user::User,
    identity: &db::user_db::SyncIdentity,
    group: i64,
    writer: &db::sync_transport::WriterRegistration,
) {
    let NameState::Ready { token, .. } =
        db::causal_group_lifecycle::get_state(&source.db, user, identity, group)
            .await
            .unwrap()
            .name
    else {
        panic!("expected authored rename")
    };
    let dot = &token.frontier[0];
    let op = db::causal_group_lifecycle::export_operation(&source.db, user, identity, dot)
        .await
        .unwrap();
    let proof = db::sync_transport::writer_proof(&source.db, user, identity, dot)
        .await
        .unwrap();
    let body = serde_json::json!({"protocol":1,"connection_id":proof.connection_id,"writer_nonce":proof.writer_nonce,"item":db::sync_transport::encode_operation(&op).unwrap()});
    let result = router
        .clone()
        .oneshot(post(
            "/api/v2/sync/items",
            &body,
            Some(cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
}

async fn publish_model_move_operation(
    router: &Router,
    cookie: &str,
    source: &service::AppState,
    user: &db::model::user::User,
    identity: &db::user_db::SyncIdentity,
    model: i64,
    writer: &db::sync_transport::WriterRegistration,
) {
    let MoveState::Ready { token, .. } =
        db::causal_model_move::get_state(&source.db, user, identity, model)
            .await
            .unwrap()
    else {
        panic!("expected authored move")
    };
    let dot = &token.frontier[0];
    let op = db::causal_group_lifecycle::export_operation(&source.db, user, identity, dot)
        .await
        .unwrap();
    let proof = db::sync_transport::writer_proof(&source.db, user, identity, dot)
        .await
        .unwrap();
    let body = serde_json::json!({"protocol":1,"connection_id":proof.connection_id,"writer_nonce":proof.writer_nonce,"item":db::sync_transport::encode_operation(&op).unwrap()});
    let result = router
        .clone()
        .oneshot(post(
            "/api/v2/sync/items",
            &body,
            Some(cookie),
            Some(&writer.writer_secret),
        ))
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Two offline destination branches and a later source move exercise choice freshness.
async fn occupied_common_baseline_preserves_concurrent_model_destinations() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, source, global, _, source_writer) =
        publish_observed_source_with_trash(&router, &cookie, false).await;
    let (receiver_dir, receiver) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let si = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let ri = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let source_group: i64 = sqlx::query_scalar(
        "SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&*source.db)
    .await
    .unwrap();
    let source_model: i64 = sqlx::query_scalar(
        "SELECT model_id FROM models WHERE model_user_id=? AND model_group_id=?",
    )
    .bind(user.id)
    .bind(source_group)
    .fetch_one(&*source.db)
    .await
    .unwrap();
    let bundle =
        db::sync_transport::observations::reload_source(&source.db, &user, &si, source_group)
            .await
            .unwrap()
            .unwrap();
    let file = receiver_dir.path().join("existing.stl");
    std::fs::write(&file, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &receiver.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        Some(file.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let bindings: Vec<_> = bundle
        .model_envelopes()
        .iter()
        .map(|event| db::causal_adoption::ModelBlobBinding {
            entity: event.entity.clone(),
            local_blob_id: blob,
        })
        .collect();
    let peer = db::sync_peer::bind_peer(&receiver.db, &user, &ri, &si)
        .await
        .unwrap();
    let installed = db::causal_adoption::install_observed_populated_group(
        &receiver.db,
        &user,
        &ri,
        &peer,
        &bundle,
        &bindings,
    )
    .await
    .unwrap();
    let receiver_model = installed.models[0].1;
    for destination in [None, Some(source_group)] {
        db::group_db::set_group_id_on_models(
            &source.db,
            &user,
            destination,
            vec![source_model],
            None,
        )
        .await
        .unwrap();
        publish_model_move_operation(
            &router,
            &cookie,
            &source,
            &user,
            &si,
            source_model,
            &source_writer,
        )
        .await;
    }
    let receiver_writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(&receiver.db, &user, &ri, &receiver_writer.server_identity)
        .await
        .unwrap();
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &ri, &receiver_writer)
        .await
        .unwrap();
    let local_group = db::group_db::add_empty_group(&receiver.db, &user, "local destination", None)
        .await
        .unwrap();
    let local_root =
        db::causal_group_lifecycle::enroll_pristine_group(&receiver.db, &user, &ri, local_group.id)
            .await
            .unwrap();
    db::group_db::set_group_id_on_models(
        &receiver.db,
        &user,
        Some(local_group.id),
        vec![receiver_model],
        None,
    )
    .await
    .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &ri,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert_eq!(
        client.pull_next(&receiver, &user, &ri).await.unwrap(),
        PullOutcome::AlreadyApplied {
            sequence: "1".into()
        }
    );
    assert_eq!(
        client.pull_next(&receiver, &user, &ri).await.unwrap(),
        PullOutcome::Applied {
            sequence: "2".into()
        }
    );
    let MoveState::Conflict {
        choices,
        token: old,
    } = db::causal_model_move::get_state(&receiver.db, &user, &ri, receiver_model)
        .await
        .unwrap()
    else {
        panic!("expected different offline destinations")
    };
    assert_eq!(choices.len(), 2);
    assert!(choices.iter().any(|choice| choice.destination.is_none()));
    assert!(
        choices
            .iter()
            .any(|choice| choice.destination.as_ref() == Some(local_root.entity()))
    );
    assert_eq!(
        client.pull_next(&receiver, &user, &ri).await.unwrap(),
        PullOutcome::Applied {
            sequence: "3".into()
        }
    );
    assert!(matches!(
        db::causal_model_move::resolve(
            &receiver.db,
            &user,
            &ri,
            receiver_model,
            &old,
            Some(local_root.entity().clone())
        )
        .await
        .unwrap(),
        MoveResolveResult::Stale(_)
    ));
    let MoveState::Conflict { choices, token } =
        db::causal_model_move::get_state(&receiver.db, &user, &ri, receiver_model)
            .await
            .unwrap()
    else {
        panic!("expected updated destination choices")
    };
    assert!(
        choices
            .iter()
            .any(|choice| choice.destination.as_ref() == Some(local_root.entity()))
    );
    assert!(choices.iter().any(|choice| {
        choice
            .destination
            .as_ref()
            .is_some_and(|entity| entity.global_id == global)
    }));
    assert!(matches!(
        db::causal_model_move::resolve(
            &receiver.db,
            &user,
            &ri,
            receiver_model,
            &token,
            Some(local_root.entity().clone())
        )
        .await
        .unwrap(),
        MoveResolveResult::Resolved(MoveState::Ready { .. })
    ));
    assert_eq!(std::fs::read(&file).unwrap(), b"abc");
}

#[cfg(unix)] // inode identity assertions need std::os::unix::fs::MetadataExt
#[tokio::test]
#[allow(clippy::too_many_lines)] // A second authenticated hub has a separate server identity but carries the identical immutable source log.
async fn second_authenticated_hub_reuses_only_matching_content_attestation() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, _source, global, _, _) = publish_observed_source(&router, &cookie).await;
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let first_writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(
        &receiver.db,
        &user,
        &identity,
        &first_writer.server_identity,
    )
    .await
    .unwrap();
    let first = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &first.origin(), false)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &first.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &first_writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &identity,
        &first.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    assert_eq!(
        client.pull_next(&receiver, &user, &identity).await.unwrap(),
        PullOutcome::Applied {
            sequence: "1".into()
        }
    );
    let local_group: i64 = sqlx::query_scalar(
        "SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&*receiver.db)
    .await
    .unwrap();
    db::group_db::edit_group(&receiver.db, &user, local_group, "local edit", None, None)
        .await
        .unwrap();
    let second_dir = tempfile::TempDir::new().unwrap();
    let second_path = second_dir.path().join("db.sqlite");
    sqlx::query("VACUUM INTO ?")
        .bind(second_path.to_string_lossy().into_owned())
        .execute(&*hub.db)
        .await
        .unwrap();
    let backup = second_dir.path().join("backups");
    std::fs::create_dir_all(&backup).unwrap();
    let second_db = db::db_context::setup_db(&second_path, &backup).await;
    sqlx::query("UPDATE sync_instance SET instance_id=? WHERE singleton=1")
        .bind(db::random_hex_32())
        .execute(&second_db)
        .await
        .unwrap();
    // The copied ledger certifies the first hub's managed files; keep those
    // exact files while the copied database uses a distinct server identity.
    let data_path = hub.get_configuration().data_path;
    let copied_slots: Vec<String> =
        sqlx::query_scalar("SELECT stage_path FROM sync_stage_slots ORDER BY stage_path")
            .fetch_all(&second_db)
            .await
            .unwrap();
    let copied_intents: Vec<String> =
        sqlx::query_scalar("SELECT stage_path FROM sync_transfer_intents ORDER BY stage_path")
            .fetch_all(&second_db)
            .await
            .unwrap();
    let copied_files: Vec<_> = copied_slots
        .iter()
        .filter_map(|path| {
            std::fs::metadata(path)
                .ok()
                .map(|meta| (path.clone(), meta.dev(), meta.ino()))
        })
        .collect();
    let second_db = Arc::new(second_db);
    let second_state = service::AppState {
        db: second_db.clone(),
        configuration: std::sync::Mutex::new(service::Configuration {
            data_path: data_path.clone(),
            ..Default::default()
        }),
        import_mutex: Arc::new(tokio::sync::Mutex::new(())),
        app_data_path: data_path,
    };
    let second_router = crate::test_support::with_session(
        crate::controller::auth_controller::login_router().merge(super::super::router()),
        second_state,
    )
    .await;
    let login = second_router
        .clone()
        .oneshot(post(
            "/api/v1/login/token",
            &serde_json::json!({"token":token,"group_deletion_protocol":1}),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::NO_CONTENT);
    let second_cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let second_writer = registration(&second_router, &second_cookie, &receiver).await;
    assert_ne!(
        first_writer.server_identity.instance_id,
        second_writer.server_identity.instance_id
    );
    let blob_request = Request::get(format!("/api/v2/sync/observations/1/blobs/ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad?protocol=1&connection_id={}",second_writer.connection_id)).header(header::COOKIE,&second_cookie).header("x-sync-writer-secret",&second_writer.writer_secret).body(Body::empty()).unwrap();
    let blob_response = second_router.clone().oneshot(blob_request).await.unwrap();
    let blob_status = blob_response.status();
    let blob_bytes = to_bytes(blob_response.into_body(), 1024).await.unwrap();
    assert_eq!(
        blob_status,
        StatusCode::OK,
        "second hub must serve the baseline bytes: {}",
        String::from_utf8_lossy(&blob_bytes)
    );
    assert_eq!(&blob_bytes[..], b"abc");
    let remaining_slots: Vec<String> =
        sqlx::query_scalar("SELECT stage_path FROM sync_stage_slots ORDER BY stage_path")
            .fetch_all(&*second_db)
            .await
            .unwrap();
    let remaining_intents: Vec<String> = sqlx::query_scalar("SELECT stage_path FROM sync_transfer_intents WHERE stage_path IN (SELECT stage_path FROM sync_stage_slots WHERE database_path != ?) ORDER BY stage_path")
        .bind(second_path.to_string_lossy().into_owned()).fetch_all(&*second_db).await.unwrap();
    assert!(
        copied_slots
            .iter()
            .all(|path| remaining_slots.contains(path))
    );
    assert_eq!(remaining_intents, copied_intents);
    assert!(
        remaining_slots.len() > copied_slots.len(),
        "the second database must allocate its own certified slot"
    );
    for (path, device, inode) in copied_files {
        let meta = std::fs::metadata(path).unwrap();
        assert_eq!((meta.dev(), meta.ino()), (device, inode));
    }
    db::sync_peer::bind_peer(
        &receiver.db,
        &user,
        &identity,
        &second_writer.server_identity,
    )
    .await
    .unwrap();
    let second = TlsHub::serve(second_router).await;
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &second.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &second_writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &identity,
        &second.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    sqlx::query("DROP TRIGGER immutable_sync_observation_content")
        .execute(&*receiver.db)
        .await
        .unwrap();
    let sha: String =
        sqlx::query_scalar("SELECT sha256 FROM sync_observation_contents WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    sqlx::query("UPDATE sync_observation_contents SET sha256=? WHERE user_id=?")
        .bind("f".repeat(64))
        .bind(user.id)
        .execute(&*receiver.db)
        .await
        .unwrap();
    assert!(
        client.pull_next(&receiver, &user, &identity).await.is_err(),
        "mismatched content attestation must not be reused"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        1
    );
    sqlx::query("UPDATE sync_observation_contents SET sha256=? WHERE user_id=?")
        .bind(sha)
        .bind(user.id)
        .execute(&*receiver.db)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER immutable_sync_observation_content BEFORE UPDATE ON sync_observation_contents WHEN NEW.user_id IS NOT OLD.user_id OR NEW.batch_id IS NOT OLD.batch_id OR NEW.hash IS NOT OLD.hash OR NEW.filetype IS NOT OLD.filetype OR NEW.size IS NOT OLD.size OR NEW.sha256 IS NOT OLD.sha256 OR NEW.blob_id IS NOT NULL BEGIN SELECT RAISE(ABORT,'verified content reference is immutable'); END").execute(&*receiver.db).await.unwrap();
    assert_eq!(
        client.pull_next(&receiver, &user, &identity).await.unwrap(),
        PullOutcome::AlreadyApplied {
            sequence: "1".into()
        }
    );
    let name: String = sqlx::query_scalar("SELECT group_name FROM models_group WHERE group_id=?")
        .bind(local_group)
        .fetch_one(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(name, "local edit");
    let counts:(i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM sync_observation_contents),(SELECT COUNT(*) FROM sync_pull_receipts),(SELECT COUNT(*) FROM models)").fetch_one(&*receiver.db).await.unwrap();
    assert_eq!(counts, (1, 2, 1));
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Each unsupported local change must survive a blocked first proof.
async fn occupied_baseline_never_overwrites_unsupported_local_drift() {
    for mode in ["metadata", "label", "project", "trash"] {
        let (_hub_dir, hub, router, cookie) = app().await;
        let (_source_dir, source, global, _, _) =
            publish_observed_source_with_trash(&router, &cookie, false).await;
        let (receiver_dir, receiver) = crate::test_support::app_state().await;
        let user = db::model::user::User::default();
        let si = db::user_db::get_sync_identity(&source.db, user.id)
            .await
            .unwrap()
            .unwrap();
        let ri = db::user_db::get_sync_identity(&receiver.db, user.id)
            .await
            .unwrap()
            .unwrap();
        let group: i64 = sqlx::query_scalar(
            "SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
        )
        .bind(user.id)
        .bind(&global)
        .fetch_one(&*source.db)
        .await
        .unwrap();
        let bundle = db::sync_transport::observations::reload_source(&source.db, &user, &si, group)
            .await
            .unwrap()
            .unwrap();
        let file = receiver_dir.path().join("existing.stl");
        std::fs::write(&file, b"abc").unwrap();
        let blob = db::blob_db::add_blob(
            &receiver.db,
            "ba7816bf8f01cfea414140de5dae2223",
            "stl",
            3,
            Some(file.to_string_lossy().into_owned()),
        )
        .await
        .unwrap();
        let bindings: Vec<_> = bundle
            .model_envelopes()
            .iter()
            .map(|event| db::causal_adoption::ModelBlobBinding {
                entity: event.entity.clone(),
                local_blob_id: blob,
            })
            .collect();
        let peer = db::sync_peer::bind_peer(&receiver.db, &user, &ri, &si)
            .await
            .unwrap();
        let installed = db::causal_adoption::install_observed_populated_group(
            &receiver.db,
            &user,
            &ri,
            &peer,
            &bundle,
            &bindings,
        )
        .await
        .unwrap();
        let model = installed.models[0].1;
        let label: i64 = sqlx::query_scalar("SELECT label_id FROM models_labels WHERE model_id=?")
            .bind(model)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
        let project: i64 =
            sqlx::query_scalar("SELECT group_resource_id FROM models_group WHERE group_id=?")
                .bind(installed.group_id)
                .fetch_one(&*receiver.db)
                .await
                .unwrap();
        match mode {
            "metadata" => db::model_db::edit_model(
                &receiver.db,
                &user,
                model,
                "part",
                None,
                Some("local description"),
                db::model::ModelFlags::empty(),
                None,
                None,
            )
            .await
            .unwrap(),
            "label" => {
                db::label_db::edit_label(&receiver.db, &user, label, "local label", 13, None, None)
                    .await
                    .unwrap();
            }
            "project" => db::resource_db::edit_resource(
                &receiver.db,
                &user,
                project,
                "local project",
                db::model::resource::ResourceFlags::empty(),
                None,
                None,
            )
            .await
            .unwrap(),
            "trash" => db::model_db::trash_models(&receiver.db, &user, &[model])
                .await
                .unwrap(),
            _ => unreachable!(),
        }
        let before:(Option<String>,String,String,Option<String>)=sqlx::query_as("SELECT m.model_desc,l.label_name,r.resource_name,m.model_deleted FROM models m JOIN models_labels ml ON ml.model_id=m.model_id JOIN labels l ON l.label_id=ml.label_id JOIN models_group g ON g.group_id=m.model_group_id JOIN resources r ON r.resource_id=g.group_resource_id WHERE m.model_id=?").bind(model).fetch_one(&*receiver.db).await.unwrap();
        let writer = registration(&router, &cookie, &receiver).await;
        db::sync_peer::bind_peer(&receiver.db, &user, &ri, &writer.server_identity)
            .await
            .unwrap();
        let server = TlsHub::serve(router).await;
        let token = db::random_hex_32();
        let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
            .await
            .unwrap()
            .unwrap();
        db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
            .await
            .unwrap();
        db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
            .await
            .unwrap();
        db::sync_transport::install_authority(&receiver.db, &user, &ri, &writer)
            .await
            .unwrap();
        let client = AuthenticatedHub::connect(
            &receiver,
            &user,
            &ri,
            &server.origin(),
            &token,
            HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
        )
        .await
        .unwrap();
        assert!(
            client.pull_next(&receiver, &user, &ri).await.is_err(),
            "unsupported {mode} drift must block without acknowledgment"
        );
        let after:(Option<String>,String,String,Option<String>)=sqlx::query_as("SELECT m.model_desc,l.label_name,r.resource_name,m.model_deleted FROM models m JOIN models_labels ml ON ml.model_id=m.model_id JOIN labels l ON l.label_id=ml.label_id JOIN models_group g ON g.group_id=m.model_group_id JOIN resources r ON r.resource_id=g.group_resource_id WHERE m.model_id=?").bind(model).fetch_one(&*receiver.db).await.unwrap();
        assert_eq!(before, after, "{mode} local edit must survive");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(std::fs::read(file).unwrap(), b"abc");
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Real baseline and independently authored branches must share one checked identity.
async fn occupied_common_baseline_keeps_independent_branches_and_stale_choices() {
    let (_hub_dir, hub, router, cookie) = app().await;
    let (_source_dir, source, global, deleted, source_writer) =
        publish_observed_source(&router, &cookie).await;
    let (receiver_dir, receiver) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let source_identity = db::user_db::get_sync_identity(&source.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let receiver_identity = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let source_group: i64 = sqlx::query_scalar(
        "SELECT group_id FROM models_group WHERE group_user_id=? AND group_unique_global_id=?",
    )
    .bind(user.id)
    .bind(&global)
    .fetch_one(&*source.db)
    .await
    .unwrap();
    let bundle = db::sync_transport::observations::reload_source(
        &source.db,
        &user,
        &source_identity,
        source_group,
    )
    .await
    .unwrap()
    .unwrap();
    let file = receiver_dir.path().join("existing.stl");
    std::fs::write(&file, b"abc").unwrap();
    let blob = db::blob_db::add_blob(
        &receiver.db,
        "ba7816bf8f01cfea414140de5dae2223",
        "stl",
        3,
        Some(file.to_string_lossy().into_owned()),
    )
    .await
    .unwrap();
    let bindings: Vec<_> = bundle
        .model_envelopes()
        .iter()
        .map(|event| db::causal_adoption::ModelBlobBinding {
            entity: event.entity.clone(),
            local_blob_id: blob,
        })
        .collect();
    let peer = db::sync_peer::bind_peer(&receiver.db, &user, &receiver_identity, &source_identity)
        .await
        .unwrap();
    let installed = db::causal_adoption::install_observed_populated_group(
        &receiver.db,
        &user,
        &receiver_identity,
        &peer,
        &bundle,
        &bindings,
    )
    .await
    .unwrap();
    let group = installed.group_id;
    db::group_db::edit_group(&receiver.db, &user, group, "alpha", None, None)
        .await
        .unwrap();
    for name in ["bravo", "charlie"] {
        db::group_db::edit_group(&source.db, &user, source_group, name, None, None)
            .await
            .unwrap();
        publish_group_name_operation(
            &router,
            &cookie,
            &source,
            &user,
            &source_identity,
            source_group,
            &source_writer,
        )
        .await;
    }
    let receiver_writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(
        &receiver.db,
        &user,
        &receiver_identity,
        &receiver_writer.server_identity,
    )
    .await
    .unwrap();
    let (router, attack) = forge_observation_page(router);
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    let hub_user = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap();
    db::user_db::set_user_sync_token(&hub.db, hub_user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(
        &receiver.db,
        &user,
        &receiver_identity,
        &receiver_writer,
    )
    .await
    .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &receiver_identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    for mode in [1, 5] {
        attack.store(mode, Ordering::SeqCst);
        assert!(
            client
                .pull_next(&receiver, &user, &receiver_identity)
                .await
                .is_err(),
            "forged baseline proof and full hash must be rejected"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
                .fetch_one(&*receiver.db)
                .await
                .unwrap(),
            0
        );
    }
    attack.store(0, Ordering::SeqCst);
    sqlx::query("DROP TRIGGER immutable_observation_batch")
        .execute(&*receiver.db)
        .await
        .unwrap();
    let archived_label: String =
        sqlx::query_scalar("SELECT label_snapshot FROM causal_observation_batches WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&*receiver.db)
            .await
            .unwrap();
    sqlx::query("UPDATE causal_observation_batches SET label_snapshot=? WHERE user_id=?")
        .bind(format!("{archived_label} "))
        .bind(user.id)
        .execute(&*receiver.db)
        .await
        .unwrap();
    assert!(
        client
            .pull_next(&receiver, &user, &receiver_identity)
            .await
            .is_err(),
        "a different archived baseline cannot be accepted"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        0
    );
    sqlx::query("UPDATE causal_observation_batches SET label_snapshot=? WHERE user_id=?")
        .bind(archived_label)
        .bind(user.id)
        .execute(&*receiver.db)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER immutable_observation_batch BEFORE UPDATE ON causal_observation_batches BEGIN SELECT RAISE(ABORT,'observation batch is immutable'); END").execute(&*receiver.db).await.unwrap();
    std::fs::write(&file, b"abd").unwrap();
    assert!(
        client
            .pull_next(&receiver, &user, &receiver_identity)
            .await
            .is_err(),
        "the existing local blob must match the full hash"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        0
    );
    std::fs::write(&file, b"abc").unwrap();
    sqlx::query("CREATE TRIGGER fail_common_receipt BEFORE INSERT ON sync_pull_receipts BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END").execute(&*receiver.db).await.unwrap();
    assert!(
        client
            .pull_next(&receiver, &user, &receiver_identity)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pull_receipts")
            .fetch_one(&*receiver.db)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER fail_common_receipt")
        .execute(&*receiver.db)
        .await
        .unwrap();
    assert_eq!(
        client
            .pull_next(&receiver, &user, &receiver_identity)
            .await
            .unwrap(),
        PullOutcome::AlreadyApplied {
            sequence: "1".into()
        }
    );
    let row:(String,String,Option<String>,String,String,i64) = sqlx::query_as("SELECT g.group_name,m.model_name,m.model_deleted,r.resource_name,l.label_name,m.model_blob_id FROM models_group g JOIN models m ON m.model_group_id=g.group_id JOIN resources r ON r.resource_id=g.group_resource_id JOIN models_labels ml ON ml.model_id=m.model_id JOIN labels l ON l.label_id=ml.label_id WHERE g.group_id=? AND g.group_user_id=?").bind(group).bind(user.id).fetch_one(&*receiver.db).await.unwrap();
    assert_eq!(
        row,
        (
            "alpha".into(),
            "part".into(),
            Some(deleted),
            "project".into(),
            "label".into(),
            blob
        )
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"abc");
    assert_eq!(
        client
            .pull_next(&receiver, &user, &receiver_identity)
            .await
            .unwrap(),
        PullOutcome::Applied {
            sequence: "2".into()
        }
    );
    let NameState::Conflict {
        choices,
        token: old_token,
    } = db::causal_group_lifecycle::get_state(&receiver.db, &user, &receiver_identity, group)
        .await
        .unwrap()
        .name
    else {
        panic!("expected independent branches")
    };
    let mut names: Vec<_> = choices.iter().map(|choice| choice.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["alpha", "bravo"]);
    assert_eq!(
        client
            .pull_next(&receiver, &user, &receiver_identity)
            .await
            .unwrap(),
        PullOutcome::Applied {
            sequence: "3".into()
        }
    );
    assert!(matches!(
        db::causal_group_rename::resolve(
            &receiver.db,
            &user,
            &receiver_identity,
            group,
            &old_token,
            "alpha"
        )
        .await
        .unwrap(),
        ResolveResult::Stale(_)
    ));
    let NameState::Conflict { choices, token } =
        db::causal_group_lifecycle::get_state(&receiver.db, &user, &receiver_identity, group)
            .await
            .unwrap()
            .name
    else {
        panic!("expected updated conflict")
    };
    let mut names: Vec<_> = choices.iter().map(|choice| choice.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["alpha", "charlie"]);
    assert!(
        matches!(db::causal_group_rename::resolve(&receiver.db,&user,&receiver_identity,group,&token,"alpha").await.unwrap(),ResolveResult::Resolved(NameState::Ready { ref name,.. }) if name=="alpha")
    );
}

#[allow(clippy::too_many_lines)] // A live request barrier ties account changes to the pending download.
async fn assert_inflight_revocation(replacement: u8) {
    let (_hub_dir, hub, router, cookie) = app().await;
    let hub_user_id = db::user_db::get_user_by_email(&hub.db, "alice@example.com")
        .await
        .unwrap()
        .unwrap()
        .id;
    let (_source_dir, _source, _global, _, _) = publish_observed_source(&router, &cookie).await;
    let (_receiver_dir, receiver) = crate::test_support::app_state().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&receiver.db, user.id)
        .await
        .unwrap()
        .unwrap();
    let writer = registration(&router, &cookie, &receiver).await;
    db::sync_peer::bind_peer(&receiver.db, &user, &identity, &writer.server_identity)
        .await
        .unwrap();
    let reached = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let (server_reached, server_release) = (reached.clone(), release.clone());
    let router = router.layer(middleware::from_fn(
        move |request: Request<Body>, next: Next| {
            let (reached, release) = (server_reached.clone(), server_release.clone());
            async move {
                if request.uri().path().contains("/blobs/") {
                    reached.notify_one();
                    release.notified().await;
                }
                next.run(request).await
            }
        },
    ));
    let server = TlsHub::serve(router).await;
    let token = db::random_hex_32();
    db::user_db::set_user_sync_token(
        &hub.db,
        db::user_db::get_user_by_email(&hub.db, "alice@example.com")
            .await
            .unwrap()
            .unwrap()
            .id,
        &token,
        &server.origin(),
        false,
    )
    .await
    .unwrap();
    db::user_db::set_user_sync_token(&receiver.db, user.id, &token, &server.origin(), false)
        .await
        .unwrap();
    db::sync_transport::install_authority(&receiver.db, &user, &identity, &writer)
        .await
        .unwrap();
    let client = AuthenticatedHub::connect(
        &receiver,
        &user,
        &identity,
        &server.origin(),
        &token,
        HubTrust::AdditionalCaPem(TEST_CA.to_vec()),
    )
    .await
    .unwrap();
    let pull = client.pull_next(&receiver, &user, &identity);
    let revoke = async {
        reached.notified().await;
        if replacement == 1 {
            sqlx::query("DELETE FROM sync_accounts WHERE user_id=?")
                .bind(user.id)
                .execute(&*receiver.db)
                .await
                .unwrap();
            sqlx::query("INSERT INTO sync_accounts (user_id,account_id) VALUES (?,?)")
                .bind(user.id)
                .bind(db::random_hex_32())
                .execute(&*receiver.db)
                .await
                .unwrap();
        } else if replacement == 2 {
            sqlx::query("DELETE FROM sync_accounts WHERE user_id=?")
                .bind(hub_user_id)
                .execute(&*hub.db)
                .await
                .unwrap();
            sqlx::query("INSERT INTO sync_accounts (user_id,account_id) VALUES (?,?)")
                .bind(hub_user_id)
                .bind(db::random_hex_32())
                .execute(&*hub.db)
                .await
                .unwrap();
        } else {
            db::user_db::set_user_sync_token(
                &receiver.db,
                user.id,
                &db::random_hex_32(),
                &server.origin(),
                false,
            )
            .await
            .unwrap();
            db::user_db::set_user_sync_token(
                &receiver.db,
                user.id,
                &token,
                &server.origin(),
                false,
            )
            .await
            .unwrap();
        }
        release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(pull, revoke)
    })
    .await
    .expect("download must reach the test barrier");
    assert!(
        result.is_err(),
        "a revoked session must remain revoked after its settings are restored"
    );
    let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM models),(SELECT COUNT(*) FROM models_group),(SELECT COUNT(*) FROM sync_pull_receipts)").fetch_one(&*receiver.db).await.unwrap();
    assert_eq!(counts, (0, 0, 0));
}
