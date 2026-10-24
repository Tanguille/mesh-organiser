//! User deletion must cascade through owned rows and clean up only unreferenced blobs.

use std::fs;

use db::{blob_db, label_db, model_db, resource_db, share_db, user_db};

mod common;

#[tokio::test]
async fn delete_user_removes_owned_data_and_preserves_shared_blob_files() {
    let (_dir, app_state) = common::test_app_state().await;
    let db = &app_state.db;
    let survivor = db::model::user::User::default();
    let deleted_id = user_db::add_user(db, "deleted", "deleted@example.com", &db::random_hex_32())
        .await
        .unwrap();
    let deleted = user_db::get_user_by_id(db, deleted_id)
        .await
        .unwrap()
        .unwrap();
    let model_dir = app_state.get_model_dir();
    let image_dir = app_state.get_image_dir();

    for (hash, owner) in [
        ("removed", &deleted),
        ("surviving", &survivor),
        ("shared", &deleted),
    ] {
        let blob_id = blob_db::add_blob(db, hash, "stl", 5, None).await.unwrap();
        model_db::add_model(db, owner, hash, blob_id, None, None)
            .await
            .unwrap();
        fs::write(model_dir.join(format!("{hash}.stl")), b"model").unwrap();
        fs::write(image_dir.join(format!("{hash}.png")), b"thumbnail").unwrap();

        if hash == "shared" {
            model_db::add_model(db, &survivor, hash, blob_id, None, None)
                .await
                .unwrap();
        }
    }
    label_db::add_label(db, &deleted, "label", 0, None)
        .await
        .unwrap();
    resource_db::add_resource(db, &deleted, "resource", None)
        .await
        .unwrap();
    share_db::create_share(db, &deleted, "share").await.unwrap();

    service::user_service::delete_user(&app_state, deleted_id)
        .await
        .unwrap();

    assert!(
        user_db::get_user_by_id(db, deleted_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        user_db::get_user_by_id(db, survivor.id)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        model_db::get_model_count(db, &deleted, None).await.unwrap(),
        0
    );
    assert_eq!(
        model_db::get_model_count(db, &survivor, None)
            .await
            .unwrap(),
        2
    );
    assert!(
        label_db::get_labels_min(db, &deleted)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        resource_db::get_resources(db, &deleted)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(share_db::get_shares(db, &deleted).await.unwrap().is_empty());
    assert!(
        blob_db::get_blob_via_sha256(db, "removed")
            .await
            .unwrap()
            .is_none()
    );
    assert!(!model_dir.join("removed.stl").exists());
    assert!(!image_dir.join("removed.png").exists());

    for hash in ["surviving", "shared"] {
        assert!(
            blob_db::get_blob_via_sha256(db, hash)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            fs::read(model_dir.join(format!("{hash}.stl"))).unwrap(),
            b"model"
        );
        assert_eq!(
            fs::read(image_dir.join(format!("{hash}.png"))).unwrap(),
            b"thumbnail"
        );
    }
}
