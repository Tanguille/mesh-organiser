//! Re-extracting an imported 3MF directory must reuse its models and existing group.

use std::fs;

use db::{group_db, model::user::User, model_db};
use service::import_state::ImportState;

mod common;

#[tokio::test]
async fn repeated_extracted_directory_import_returns_the_existing_group_without_regrouping() {
    let (_dir, app_state) = common::test_app_state().await;
    let user = User::default();
    let source = tempfile::tempdir().unwrap();
    let extracted = source.path().join("Extracted_Parts");
    fs::create_dir(&extracted).unwrap();
    // import_path stores these bytes without parsing them or generating thumbnails.
    fs::write(extracted.join("first.stl"), b"first extracted mesh").unwrap();
    fs::write(extracted.join("second.stl"), b"second extracted mesh").unwrap();
    let import = || {
        service::import_service::import_path(
            extracted.to_str().unwrap(),
            &app_state,
            ImportState::new(None, false, false, false, user.clone()),
        )
    };
    let first = import().await.unwrap();
    let first_group_id = first.imported_models[0].group_id.unwrap();
    let mut original_ids = first.all_model_ids();
    original_ids.sort_unstable();
    assert_eq!(original_ids.len(), 2);
    let original_models = model_db::get_models_via_ids(&app_state.db, &user, original_ids.clone())
        .await
        .unwrap();
    let original_groups: Vec<_> = original_models
        .iter()
        .map(|model| serde_json::to_value(model.group.as_ref().unwrap()).unwrap())
        .collect();

    let fresh_group = service::group_meta_from_import(&first, &app_state)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(fresh_group).unwrap(),
        original_groups[0]
    );

    let repeated = import().await.unwrap();

    let mut repeated_ids = repeated.all_model_ids();
    repeated_ids.sort_unstable();
    assert_eq!(repeated_ids, original_ids);
    let models = model_db::get_models_via_ids(&app_state.db, &user, repeated_ids)
        .await
        .unwrap();
    let groups: Vec<_> = models
        .iter()
        .map(|model| serde_json::to_value(model.group.as_ref().unwrap()).unwrap())
        .collect();
    assert_eq!(groups, original_groups);
    let group_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models_group")
        .fetch_one(&*app_state.db)
        .await
        .unwrap();
    assert_eq!(group_count, 1);

    let group = service::group_meta_from_import(&repeated, &app_state)
        .await
        .expect("deduplicating extraction should return its models' existing group");

    assert_eq!(group.id, first_group_id);
    assert_eq!(serde_json::to_value(group).unwrap(), original_groups[0]);

    let other_group = group_db::add_empty_group(&app_state.db, &user, "Other group", None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(
        &app_state.db,
        &user,
        Some(other_group.id),
        vec![original_ids[1]],
        None,
    )
    .await
    .unwrap();
    let before_mixed = model_db::get_models_via_ids(&app_state.db, &user, original_ids.clone())
        .await
        .unwrap();
    let expected_groups: Vec<_> = before_mixed
        .iter()
        .map(|model| serde_json::to_value(model.group.as_ref().unwrap()).unwrap())
        .collect();

    let mixed = import().await.unwrap();
    let group = service::group_meta_from_import(&mixed, &app_state)
        .await
        .unwrap();

    let first_imported_id = mixed.all_model_ids()[0];
    let expected_group = before_mixed
        .iter()
        .find(|model| model.id == first_imported_id)
        .unwrap()
        .group
        .as_ref()
        .unwrap();
    assert_eq!(
        serde_json::to_value(group).unwrap(),
        serde_json::to_value(expected_group).unwrap()
    );
    let after_mixed = model_db::get_models_via_ids(&app_state.db, &user, original_ids)
        .await
        .unwrap();
    let actual_groups: Vec<_> = after_mixed
        .iter()
        .map(|model| serde_json::to_value(model.group.as_ref().unwrap()).unwrap())
        .collect();
    assert_eq!(actual_groups, expected_groups);
}

#[tokio::test]
async fn group_metadata_rejects_empty_missing_and_ungrouped_models() {
    let (_dir, app_state) = common::test_app_state().await;
    let mut import_state = ImportState::new(None, false, false, false, User::default());
    assert_eq!(
        common::internal_error_message(
            service::group_meta_from_import(&import_state, &app_state).await
        ),
        "3MF extract produced no imported group"
    );
    import_state.add_model_id_to_current_set(9999);
    assert_eq!(
        common::internal_error_message(
            service::group_meta_from_import(&import_state, &app_state).await
        ),
        "3MF extract produced no imported group"
    );
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("ungrouped.stl");
    fs::write(&path, b"an ungrouped mesh").unwrap();
    let ungrouped = service::import_service::import_path(
        path.to_str().unwrap(),
        &app_state,
        ImportState::new(None, false, false, false, User::default()),
    )
    .await
    .unwrap();

    assert_eq!(
        common::internal_error_message(
            service::group_meta_from_import(&ungrouped, &app_state).await
        ),
        "3MF extract produced no imported group"
    );
}
