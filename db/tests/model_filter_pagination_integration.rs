//! Pages and filtered totals must apply the same model predicates, including exclusions.

use db::{
    blob_db,
    db_context::DbContext,
    group_db, label_db,
    model::{ModelFlags, blob::FileType, user::User},
    model_db::{self, ModelFilterOptions},
    user_db,
};

mod common;

async fn add_model(
    db: &DbContext,
    user: &User,
    name: &str,
    filetype: &str,
    flags: ModelFlags,
) -> i64 {
    let blob_id = blob_db::add_blob(db, &db::random_hex_32(), filetype, 1, None)
        .await
        .unwrap();
    let model_id = model_db::add_model(db, user, name, blob_id, None, None)
        .await
        .unwrap();
    model_db::edit_model(db, user, model_id, name, None, None, flags, None, None)
        .await
        .unwrap();

    model_id
}

async fn assert_page(
    db: &DbContext,
    user: &User,
    options: ModelFilterOptions,
    expected_ids: &[i64],
    expected_total: i64,
) {
    let total = model_db::get_filtered_model_count(db, user, &options)
        .await
        .unwrap();
    let page = model_db::get_models(db, user, options).await.unwrap();
    let ids: Vec<i64> = page.items.into_iter().map(|model| model.id).collect();

    assert_eq!(ids, expected_ids);
    assert_eq!(total, expected_total);
}

#[tokio::test]
async fn flag_filters_and_totals_cover_all_printed_favorite_combinations() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let mut ids = Vec::new();
    for (name, flags) in [
        ("plain", ModelFlags::empty()),
        ("printed", ModelFlags::Printed),
        ("favorite", ModelFlags::Favorite),
        ("both", ModelFlags::Printed | ModelFlags::Favorite),
    ] {
        ids.push(add_model(&db, &user, name, "stl", flags).await);
    }

    // Required bits represent true, excluded bits false, and neither represents None.
    for (required, excluded, expected) in [
        (None, None, vec![ids[0], ids[1], ids[2], ids[3]]),
        (Some(ModelFlags::Printed), None, vec![ids[1], ids[3]]),
        (None, Some(ModelFlags::Printed), vec![ids[0], ids[2]]),
        (Some(ModelFlags::Favorite), None, vec![ids[2], ids[3]]),
        (None, Some(ModelFlags::Favorite), vec![ids[0], ids[1]]),
        (
            Some(ModelFlags::Printed | ModelFlags::Favorite),
            None,
            vec![ids[3]],
        ),
        (
            Some(ModelFlags::Printed),
            Some(ModelFlags::Favorite),
            vec![ids[1]],
        ),
        (
            Some(ModelFlags::Favorite),
            Some(ModelFlags::Printed),
            vec![ids[2]],
        ),
        (
            None,
            Some(ModelFlags::Printed | ModelFlags::Favorite),
            vec![ids[0]],
        ),
    ] {
        assert_page(
            &db,
            &user,
            ModelFilterOptions {
                model_flags: required,
                excluded_model_flags: excluded,
                ..Default::default()
            },
            &expected,
            i64::try_from(expected.len()).unwrap(),
        )
        .await;
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Keep the predicate cases beside their shared four-model fixture.
async fn count_and_page_agree_for_model_label_group_text_type_and_unassigned_filters() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let first = add_model(&db, &user, "needle model", "STL", ModelFlags::empty()).await;
    let second = add_model(&db, &user, "beta", "stl.zip", ModelFlags::empty()).await;
    let third = add_model(&db, &user, "gamma", "obj", ModelFlags::empty()).await;
    let fourth = add_model(&db, &user, "delta", "stp.zip", ModelFlags::empty()).await;
    model_db::edit_model(
        &db,
        &user,
        second,
        "beta",
        None,
        Some("needle description"),
        ModelFlags::empty(),
        None,
        None,
    )
    .await
    .unwrap();
    let first_group = group_db::add_empty_group(&db, &user, "ordinary group", None)
        .await
        .unwrap()
        .id;
    let third_group = group_db::add_empty_group(&db, &user, "needle group", None)
        .await
        .unwrap()
        .id;
    for (group_id, model_id) in [(first_group, first), (third_group, third)] {
        group_db::set_group_id_on_models(&db, &user, Some(group_id), vec![model_id], None)
            .await
            .unwrap();
    }
    let label = label_db::add_label(&db, &user, "label", 0, None)
        .await
        .unwrap()
        .id;
    let extra_label = label_db::add_label(&db, &user, "extra", 0, None)
        .await
        .unwrap()
        .id;
    label_db::add_labels_on_models(&db, &user, &[label], &[first, second], None)
        .await
        .unwrap();
    label_db::add_labels_on_models(&db, &user, &[extra_label], &[first], None)
        .await
        .unwrap();

    for (options, expected) in [
        (
            ModelFilterOptions {
                unlabeled_only: true,
                ..Default::default()
            },
            vec![third, fourth],
        ),
        (
            ModelFilterOptions {
                ungrouped_only: true,
                ..Default::default()
            },
            vec![second, fourth],
        ),
        (
            ModelFilterOptions {
                unlabeled_only: true,
                ungrouped_only: true,
                ..Default::default()
            },
            vec![fourth],
        ),
        (
            ModelFilterOptions {
                model_ids: Some(vec![first, fourth]),
                ..Default::default()
            },
            vec![first, fourth],
        ),
        (
            ModelFilterOptions {
                group_ids: Some(vec![first_group, third_group]),
                ..Default::default()
            },
            vec![first, third],
        ),
        (
            ModelFilterOptions {
                label_ids: Some(vec![label, extra_label]),
                ..Default::default()
            },
            vec![first, second],
        ),
        (
            ModelFilterOptions {
                file_types: Some(vec![FileType::Stl]),
                ..Default::default()
            },
            vec![first, second],
        ),
        (
            ModelFilterOptions {
                file_types: Some(vec![FileType::Step]),
                ..Default::default()
            },
            vec![fourth],
        ),
        (
            ModelFilterOptions {
                text_search: Some(String::from("needle")),
                ..Default::default()
            },
            vec![first, second, third],
        ),
        (
            ModelFilterOptions {
                group_ids: Some(vec![first_group]),
                label_ids: Some(vec![label]),
                file_types: Some(vec![FileType::Stl]),
                text_search: Some(String::from("needle")),
                ..Default::default()
            },
            vec![first],
        ),
        (
            ModelFilterOptions {
                ungrouped_only: true,
                group_ids: Some(vec![first_group]),
                ..Default::default()
            },
            vec![],
        ),
        (
            ModelFilterOptions {
                unlabeled_only: true,
                label_ids: Some(vec![label]),
                ..Default::default()
            },
            vec![],
        ),
    ] {
        assert_page(
            &db,
            &user,
            options,
            &expected,
            i64::try_from(expected.len()).unwrap(),
        )
        .await;
    }
}

#[tokio::test]
async fn filtered_total_ignores_pagination_while_pages_keep_owner_and_trash_scope() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let other_user = User {
        id: user_db::add_user(&db, "other", "other@example.com", &db::random_hex_32())
            .await
            .unwrap(),
        ..User::default()
    };
    let mut live = Vec::new();
    for _ in 0..3 {
        live.push(add_model(&db, &user, "match", "stl", ModelFlags::Printed).await);
    }
    let trashed = add_model(&db, &user, "match", "stl", ModelFlags::Printed).await;
    let foreign_live = add_model(&db, &other_user, "match", "stl", ModelFlags::Printed).await;
    let foreign_trashed = add_model(&db, &other_user, "match", "stl", ModelFlags::Printed).await;
    model_db::trash_models(&db, &user, &[trashed])
        .await
        .unwrap();
    model_db::trash_models(&db, &other_user, &[foreign_trashed])
        .await
        .unwrap();

    for (page, expected) in [(1, &live[..2]), (2, &live[2..]), (3, &live[0..0])] {
        assert_page(
            &db,
            &user,
            ModelFilterOptions {
                model_flags: Some(ModelFlags::Printed),
                text_search: Some(String::from("match")),
                order_by: Some(model_db::ModelOrderBy::NameAsc),
                page,
                page_size: 2,
                ..Default::default()
            },
            expected,
            3,
        )
        .await;
    }
    for (owner, only_trash, expected) in [
        (&user, true, vec![trashed]),
        (&other_user, false, vec![foreign_live]),
        (&other_user, true, vec![foreign_trashed]),
    ] {
        assert_page(
            &db,
            owner,
            ModelFilterOptions {
                trashed: only_trash,
                ..Default::default()
            },
            &expected,
            1,
        )
        .await;
    }
}

#[tokio::test]
async fn empty_filter_lists_match_no_page_items_and_have_zero_total() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    add_model(&db, &user, "existing", "stl", ModelFlags::empty()).await;

    for options in [
        ModelFilterOptions {
            model_ids: Some(vec![]),
            ..Default::default()
        },
        ModelFilterOptions {
            group_ids: Some(vec![]),
            ..Default::default()
        },
        ModelFilterOptions {
            label_ids: Some(vec![]),
            ..Default::default()
        },
        ModelFilterOptions {
            file_types: Some(vec![]),
            ..Default::default()
        },
    ] {
        assert_page(&db, &user, options, &[], 0).await;
    }
}
