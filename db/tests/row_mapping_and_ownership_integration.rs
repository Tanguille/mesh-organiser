//! Characterization tests pinning row mappers, last-modified timestamps, global id edits and
//! ownership checks across the db crate, so refactors of the shared query helpers stay
//! behaviour-preserving.

use db::{
    blob_db,
    db_context::DbContext,
    group_db, label_db, label_keyword_db,
    model::{
        ModelFlags,
        blob::Blob,
        resource::{ResourceFlags, ResourceMeta},
        share::Share,
        user::User,
    },
    model_db, random_hex_32, resource_db, share_db, time_now, user_db,
};

use common::test_db;

mod common;

const MISSING_ID: i64 = 999_999;
const NEW_GLOBAL_ID: &str = "0123456789abcdef0123456789abcdef";
/// One character short of and one character past the required 32.
const INVALID_GLOBAL_IDS: [&str; 2] = [
    "0123456789abcdef0123456789abcde",
    "0123456789abcdef0123456789abcdef0",
];

type BlobFields = (i64, String, String, i64, String, Option<String>);
type ResourceFields = (i64, String, u32, String, String, String);
type ShareFields = (String, String, String, i64, Vec<i64>);

async fn add_other_user(db: &DbContext) -> User {
    let user_id = user_db::add_user(db, "other", "other@example.com", &random_hex_32())
        .await
        .unwrap();

    user_db::get_user_by_id(db, user_id).await.unwrap().unwrap()
}

/// Adds a model backed by its own blob; `sha256` doubles as the model name.
async fn add_model(db: &DbContext, user: &User, sha256: &str) -> i64 {
    let blob_id = blob_db::add_blob(db, sha256, "stl", 1, None).await.unwrap();

    model_db::add_model(db, user, sha256, blob_id, None, None)
        .await
        .unwrap()
}

/// `set_resource_on_group` treats a group without models as missing, so give it one.
async fn add_group_with_model(db: &DbContext, user: &User, name: &str) -> i64 {
    let model_id = add_model(db, user, name).await;
    let group_id = group_db::add_empty_group(db, user, name, None)
        .await
        .unwrap()
        .id;
    group_db::set_group_id_on_models(db, user, Some(group_id), vec![model_id], None)
        .await
        .unwrap();

    group_id
}

async fn text_column(db: &DbContext, sql: &'static str, id: i64) -> String {
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
}

async fn label_last_modified(db: &DbContext, label_id: i64) -> String {
    text_column(
        db,
        "SELECT label_last_modified FROM labels WHERE label_id = ?",
        label_id,
    )
    .await
}

async fn group_last_modified(db: &DbContext, group_id: i64) -> String {
    text_column(
        db,
        "SELECT group_last_modified FROM models_group WHERE group_id = ?",
        group_id,
    )
    .await
}

async fn resource_last_modified(db: &DbContext, resource_id: i64) -> String {
    text_column(
        db,
        "SELECT resource_last_modified FROM resources WHERE resource_id = ?",
        resource_id,
    )
    .await
}

fn timestamp(year: u16) -> String {
    format!("{year}-01-01T00:00:00Z")
}

fn assert_fresh_timestamp(value: &str, before: &str, after: &str) {
    assert!(
        value.len() == before.len() && before <= value && value <= after,
        "timestamp ({} chars) is not a time_now() value between {before:?} and {after:?}",
        value.len()
    );
}

fn assert_row_not_found<T>(result: &Result<T, sqlx::Error>, context: &str) {
    assert!(
        matches!(result, Err(sqlx::Error::RowNotFound)),
        "expected RowNotFound: {context}"
    );
}

fn sorted_blob_fields(blobs: Vec<Blob>) -> Vec<BlobFields> {
    let mut fields: Vec<BlobFields> = blobs
        .into_iter()
        .map(|blob| {
            (
                blob.id,
                blob.sha256,
                blob.filetype,
                blob.size,
                blob.added,
                blob.disk_path,
            )
        })
        .collect();
    fields.sort();

    fields
}

fn resource_fields(resource: &ResourceMeta) -> ResourceFields {
    (
        resource.id,
        resource.name.clone(),
        resource.flags.bits(),
        resource.created.clone(),
        resource.last_modified.clone(),
        resource.unique_global_id.clone(),
    )
}

fn share_fields(share: &Share) -> ShareFields {
    let mut model_ids = share.model_ids.clone();
    model_ids.sort_unstable();

    (
        share.id.clone(),
        share.created_at.clone(),
        share.share_name.clone(),
        share.user_id,
        model_ids,
    )
}

#[tokio::test]
async fn blob_getters_map_every_column_identically() {
    let (_dir, db) = test_db().await;
    let on_disk_id = blob_db::add_blob(
        &db,
        "on-disk-sha",
        "stl.zip",
        123,
        Some("/blobs/on-disk".to_string()),
    )
    .await
    .unwrap();
    let in_database_id = blob_db::add_blob(&db, "in-database-sha", "3mf", 456, None)
        .await
        .unwrap();
    for (blob_id, added) in [
        (on_disk_id, timestamp(2001)),
        (in_database_id, timestamp(2002)),
    ] {
        sqlx::query("UPDATE blobs SET blob_added = ? WHERE blob_id = ?")
            .bind(added)
            .bind(blob_id)
            .execute(&db)
            .await
            .unwrap();
    }
    let on_disk = (
        on_disk_id,
        "on-disk-sha".to_string(),
        "stl.zip".to_string(),
        123,
        timestamp(2001),
        Some("/blobs/on-disk".to_string()),
    );
    let in_database = (
        in_database_id,
        "in-database-sha".to_string(),
        "3mf".to_string(),
        456,
        timestamp(2002),
        None,
    );

    let all = sorted_blob_fields(blob_db::get_blobs(&db).await.unwrap());
    let via_ids = sorted_blob_fields(
        blob_db::get_blobs_via_ids(&db, vec![in_database_id, on_disk_id, MISSING_ID])
            .await
            .unwrap(),
    );
    let via_sha256 = sorted_blob_fields(
        blob_db::get_blob_via_sha256(&db, "on-disk-sha")
            .await
            .unwrap()
            .into_iter()
            .collect(),
    );

    assert_eq!(all, vec![on_disk.clone(), in_database.clone()]);
    assert_eq!(via_ids, vec![on_disk.clone(), in_database]);
    assert_eq!(via_sha256, vec![on_disk]);
    assert!(
        blob_db::get_blob_via_sha256(&db, "missing-sha")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn get_and_delete_dead_blobs_returns_and_removes_only_unreferenced_blobs() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let own_blob_id = blob_db::add_blob(&db, "own-referenced", "stl", 1, None)
        .await
        .unwrap();
    model_db::add_model(&db, &user, "own", own_blob_id, None, None)
        .await
        .unwrap();
    let foreign_blob_id = blob_db::add_blob(&db, "foreign-referenced", "obj", 2, None)
        .await
        .unwrap();
    model_db::add_model(&db, &other_user, "foreign", foreign_blob_id, None, None)
        .await
        .unwrap();
    let dead_on_disk_id = blob_db::add_blob(
        &db,
        "dead-on-disk",
        "gcode",
        3,
        Some("/blobs/dead".to_string()),
    )
    .await
    .unwrap();
    let dead_in_database_id = blob_db::add_blob(&db, "dead-in-database", "step", 4, None)
        .await
        .unwrap();
    sqlx::query("UPDATE blobs SET blob_added = ?")
        .bind(timestamp(2001))
        .execute(&db)
        .await
        .unwrap();

    let dead = sorted_blob_fields(blob_db::get_and_delete_dead_blobs(&db).await.unwrap());

    assert_eq!(
        dead,
        vec![
            (
                dead_on_disk_id,
                "dead-on-disk".to_string(),
                "gcode".to_string(),
                3,
                timestamp(2001),
                Some("/blobs/dead".to_string()),
            ),
            (
                dead_in_database_id,
                "dead-in-database".to_string(),
                "step".to_string(),
                4,
                timestamp(2001),
                None,
            ),
        ]
    );
    let remaining_ids: Vec<i64> = sorted_blob_fields(blob_db::get_blobs(&db).await.unwrap())
        .into_iter()
        .map(|fields| fields.0)
        .collect();
    assert_eq!(remaining_ids, vec![own_blob_id, foreign_blob_id]);
    assert!(
        blob_db::get_and_delete_dead_blobs(&db)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn resource_getters_map_every_column_identically() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let completed = resource_db::add_resource(&db, &user, "b-completed", None)
        .await
        .unwrap();
    resource_db::edit_resource(
        &db,
        &user,
        completed.id,
        "b-completed",
        ResourceFlags::Completed,
        Some(&timestamp(2001)),
        Some(NEW_GLOBAL_ID),
    )
    .await
    .unwrap();
    let plain = resource_db::add_resource(&db, &user, "a-plain", Some(&timestamp(2002)))
        .await
        .unwrap();
    let foreign = resource_db::add_resource(&db, &other_user, "foreign", None)
        .await
        .unwrap();
    let expected_completed = (
        completed.id,
        "b-completed".to_string(),
        ResourceFlags::Completed.bits(),
        completed.created.clone(),
        timestamp(2001),
        NEW_GLOBAL_ID.to_string(),
    );
    let expected_plain = (
        plain.id,
        "a-plain".to_string(),
        0,
        plain.created.clone(),
        timestamp(2002),
        plain.unique_global_id.clone(),
    );

    let listed: Vec<ResourceFields> = resource_db::get_resources(&db, &user)
        .await
        .unwrap()
        .iter()
        .map(resource_fields)
        .collect();
    let completed_by_id = resource_db::get_resource_meta_by_id(&db, &user, completed.id)
        .await
        .unwrap()
        .map(|resource| resource_fields(&resource));

    assert_eq!(listed, vec![expected_plain, expected_completed.clone()]);
    assert_eq!(completed_by_id, Some(expected_completed));
    for resource_id in [foreign.id, MISSING_ID] {
        assert!(
            resource_db::get_resource_meta_by_id(&db, &user, resource_id)
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn get_group_id_to_resource_map_contains_only_the_users_linked_groups() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let resource = resource_db::add_resource(&db, &user, "resource", None)
        .await
        .unwrap();
    let linked_group_id = add_group_with_model(&db, &user, "linked").await;
    add_group_with_model(&db, &user, "unlinked").await;
    resource_db::set_resource_on_group(
        &db,
        &user,
        Some(resource.id),
        linked_group_id,
        Some(&timestamp(2001)),
    )
    .await
    .unwrap();
    let foreign_resource = resource_db::add_resource(&db, &other_user, "foreign", None)
        .await
        .unwrap();
    let foreign_group_id = add_group_with_model(&db, &other_user, "foreign").await;
    resource_db::set_resource_on_group(
        &db,
        &other_user,
        Some(foreign_resource.id),
        foreign_group_id,
        None,
    )
    .await
    .unwrap();

    let map = resource_db::get_group_id_to_resource_map(&db, &user)
        .await
        .unwrap();

    assert_eq!(
        map.keys().copied().collect::<Vec<_>>(),
        vec![linked_group_id]
    );
    assert_eq!(
        resource_fields(&map[&linked_group_id]),
        (
            resource.id,
            "resource".to_string(),
            0,
            resource.created.clone(),
            timestamp(2001),
            resource.unique_global_id.clone(),
        )
    );
}

#[tokio::test]
async fn share_getters_agree_for_zero_one_and_many_models() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let first = add_model(&db, &user, "first").await;
    let second = add_model(&db, &user, "second").await;
    let third = add_model(&db, &user, "third").await;
    let before = time_now();
    let empty = share_db::create_share(&db, &user, "empty").await.unwrap();
    let single = share_db::create_share(&db, &user, "single").await.unwrap();
    share_db::set_model_ids_on_share(&db, &user, &single, vec![second])
        .await
        .unwrap();
    let many = share_db::create_share(&db, &user, "many").await.unwrap();
    share_db::set_model_ids_on_share(&db, &user, &many, vec![third, first, second])
        .await
        .unwrap();
    let after = time_now();
    let foreign = share_db::create_share(&db, &other_user, "foreign")
        .await
        .unwrap();

    let listed = share_db::get_shares(&db, &user).await.unwrap();

    let mut listed_ids: Vec<&str> = listed.iter().map(|share| share.id.as_str()).collect();
    listed_ids.sort_unstable();
    let mut expected_ids = vec![empty.as_str(), single.as_str(), many.as_str()];
    expected_ids.sort_unstable();
    assert_eq!(listed_ids, expected_ids);
    for (share_id, name, model_ids) in [
        (&empty, "empty", vec![]),
        (&single, "single", vec![second]),
        (&many, "many", vec![first, second, third]),
    ] {
        let via_id = share_fields(&share_db::get_share_via_id(&db, share_id).await.unwrap());
        let from_list = share_fields(listed.iter().find(|share| &share.id == share_id).unwrap());
        assert_eq!(from_list, via_id);
        assert_fresh_timestamp(&via_id.1, &before, &after);
        assert_eq!(
            via_id,
            (
                share_id.clone(),
                via_id.1.clone(),
                name.to_string(),
                user.id,
                model_ids
            )
        );
    }
    let foreign_share = share_fields(&share_db::get_share_via_id(&db, &foreign).await.unwrap());
    assert_eq!(
        (foreign_share.2, foreign_share.3, foreign_share.4),
        ("foreign".to_string(), other_user.id, vec![])
    );
    assert_row_not_found(
        &share_db::get_share_via_id(&db, "missing-share").await,
        "missing share id",
    );
}

#[tokio::test]
async fn label_mutations_write_the_explicit_update_timestamp_only_on_own_targets() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let model_id = add_model(&db, &user, "model").await;
    let label = label_db::add_label(&db, &user, "label", 1, Some(&timestamp(2001)))
        .await
        .unwrap();
    let bystander_id = label_db::add_label(&db, &user, "bystander", 1, Some(&timestamp(2000)))
        .await
        .unwrap()
        .id;
    let foreign_id = label_db::add_label(&db, &other_user, "foreign", 1, Some(&timestamp(2000)))
        .await
        .unwrap()
        .id;
    assert_eq!(label.last_modified, timestamp(2001));
    assert_eq!(label_last_modified(&db, label.id).await, timestamp(2001));

    label_db::edit_label(
        &db,
        &user,
        label.id,
        "label",
        1,
        Some(&timestamp(2002)),
        None,
    )
    .await
    .unwrap();
    assert_eq!(label_last_modified(&db, label.id).await, timestamp(2002));
    label_db::add_labels_on_models(&db, &user, &[label.id], &[model_id], Some(&timestamp(2003)))
        .await
        .unwrap();
    assert_eq!(label_last_modified(&db, label.id).await, timestamp(2003));
    label_db::remove_labels_from_models(
        &db,
        &user,
        &[label.id],
        &[model_id],
        Some(&timestamp(2004)),
    )
    .await
    .unwrap();
    assert_eq!(label_last_modified(&db, label.id).await, timestamp(2004));
    label_db::add_labels_on_models(&db, &user, &[label.id], &[model_id], None)
        .await
        .unwrap();
    label_db::remove_all_labels_from_models(&db, &user, &[model_id], Some(&timestamp(2005)))
        .await
        .unwrap();
    assert_eq!(label_last_modified(&db, label.id).await, timestamp(2005));
    label_keyword_db::set_keywords_for_label(
        &db,
        &user,
        label.id,
        vec!["keyword".to_string()],
        Some(&timestamp(2006)),
    )
    .await
    .unwrap();
    assert_eq!(label_last_modified(&db, label.id).await, timestamp(2006));
    label_db::set_last_updated_on_labels(&db, &user, &[label.id, foreign_id], &timestamp(2007))
        .await
        .unwrap();
    assert_eq!(label_last_modified(&db, label.id).await, timestamp(2007));
    label_db::set_last_updated_on_labels(&db, &user, &[], &timestamp(2008))
        .await
        .unwrap();

    assert_eq!(label_last_modified(&db, label.id).await, timestamp(2007));
    assert_eq!(
        label_last_modified(&db, bystander_id).await,
        timestamp(2000)
    );
    assert_eq!(label_last_modified(&db, foreign_id).await, timestamp(2000));
}

#[tokio::test]
async fn group_mutations_write_the_explicit_update_timestamp_on_every_touched_group() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let model_id = add_model(&db, &user, "model").await;
    let source = group_db::add_empty_group(&db, &user, "source", Some(&timestamp(2001)))
        .await
        .unwrap();
    let target_id = group_db::add_empty_group(&db, &user, "target", Some(&timestamp(2000)))
        .await
        .unwrap()
        .id;
    let bystander_id = group_db::add_empty_group(&db, &user, "bystander", Some(&timestamp(2000)))
        .await
        .unwrap()
        .id;
    let foreign_id = group_db::add_empty_group(&db, &other_user, "foreign", Some(&timestamp(2000)))
        .await
        .unwrap()
        .id;
    assert_eq!(source.last_modified, timestamp(2001));
    assert_eq!(group_last_modified(&db, source.id).await, timestamp(2001));

    group_db::set_group_id_on_models(
        &db,
        &user,
        Some(source.id),
        vec![model_id],
        Some(&timestamp(2002)),
    )
    .await
    .unwrap();
    assert_eq!(group_last_modified(&db, source.id).await, timestamp(2002));
    group_db::edit_group(
        &db,
        &user,
        source.id,
        "source",
        Some(&timestamp(2003)),
        None,
    )
    .await
    .unwrap();
    assert_eq!(group_last_modified(&db, source.id).await, timestamp(2003));
    group_db::set_group_id_on_models(
        &db,
        &user,
        Some(target_id),
        vec![model_id],
        Some(&timestamp(2004)),
    )
    .await
    .unwrap();
    assert_eq!(group_last_modified(&db, source.id).await, timestamp(2004));
    assert_eq!(group_last_modified(&db, target_id).await, timestamp(2004));
    group_db::set_last_updated_on_groups(&db, &user, &[target_id, foreign_id], &timestamp(2005))
        .await
        .unwrap();

    assert_eq!(group_last_modified(&db, target_id).await, timestamp(2005));
    assert_eq!(group_last_modified(&db, source.id).await, timestamp(2004));
    assert_eq!(
        group_last_modified(&db, bystander_id).await,
        timestamp(2000)
    );
    assert_eq!(group_last_modified(&db, foreign_id).await, timestamp(2000));
}

#[tokio::test]
async fn resource_mutations_write_the_explicit_update_timestamp_on_every_touched_resource() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let group_id = add_group_with_model(&db, &user, "group").await;
    let resource = resource_db::add_resource(&db, &user, "resource", Some(&timestamp(2001)))
        .await
        .unwrap();
    let replacement_id =
        resource_db::add_resource(&db, &user, "replacement", Some(&timestamp(2000)))
            .await
            .unwrap()
            .id;
    let bystander_id = resource_db::add_resource(&db, &user, "bystander", Some(&timestamp(2000)))
        .await
        .unwrap()
        .id;
    let foreign_id = resource_db::add_resource(&db, &other_user, "foreign", Some(&timestamp(2000)))
        .await
        .unwrap()
        .id;
    assert_eq!(resource.last_modified, timestamp(2001));
    assert_eq!(
        resource_last_modified(&db, resource.id).await,
        timestamp(2001)
    );

    resource_db::edit_resource(
        &db,
        &user,
        resource.id,
        "resource",
        ResourceFlags::empty(),
        Some(&timestamp(2002)),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        resource_last_modified(&db, resource.id).await,
        timestamp(2002)
    );
    resource_db::set_resource_on_group(
        &db,
        &user,
        Some(resource.id),
        group_id,
        Some(&timestamp(2003)),
    )
    .await
    .unwrap();
    assert_eq!(
        resource_last_modified(&db, resource.id).await,
        timestamp(2003)
    );
    resource_db::set_resource_on_group(
        &db,
        &user,
        Some(replacement_id),
        group_id,
        Some(&timestamp(2004)),
    )
    .await
    .unwrap();
    assert_eq!(
        resource_last_modified(&db, resource.id).await,
        timestamp(2004)
    );
    assert_eq!(
        resource_last_modified(&db, replacement_id).await,
        timestamp(2004)
    );
    resource_db::set_resource_on_group(&db, &user, None, group_id, Some(&timestamp(2005)))
        .await
        .unwrap();
    assert_eq!(
        resource_last_modified(&db, replacement_id).await,
        timestamp(2005)
    );
    resource_db::set_last_updated_on_resource(&db, &user, foreign_id, &timestamp(2006))
        .await
        .unwrap();

    assert_eq!(
        resource_last_modified(&db, resource.id).await,
        timestamp(2004)
    );
    assert_eq!(
        resource_last_modified(&db, bystander_id).await,
        timestamp(2000)
    );
    assert_eq!(
        resource_last_modified(&db, foreign_id).await,
        timestamp(2000)
    );
}

#[tokio::test]
async fn label_mutations_without_update_timestamp_write_the_current_time() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let stale = timestamp(2000);
    let model_id = add_model(&db, &user, "model").await;
    let cleared_model_id = add_model(&db, &user, "cleared-model").await;
    let mut label_ids = Vec::new();
    for name in [
        "edited",
        "linked",
        "unlinked",
        "cleared",
        "keyworded",
        "parent",
    ] {
        label_ids.push(
            label_db::add_label(&db, &user, name, 0, Some(&stale))
                .await
                .unwrap()
                .id,
        );
    }
    let [edited, linked, unlinked, cleared, keyworded, parent]: [i64; 6] =
        label_ids.try_into().unwrap();
    label_db::add_labels_on_models(&db, &user, &[unlinked], &[model_id], Some(&stale))
        .await
        .unwrap();
    label_db::add_labels_on_models(&db, &user, &[cleared], &[cleared_model_id], Some(&stale))
        .await
        .unwrap();

    let before = time_now();
    let added = label_db::add_label(&db, &user, "added", 0, None)
        .await
        .unwrap();
    label_db::edit_label(&db, &user, edited, "edited", 0, None, None)
        .await
        .unwrap();
    label_db::add_labels_on_models(&db, &user, &[linked], &[model_id], None)
        .await
        .unwrap();
    label_db::remove_labels_from_models(&db, &user, &[unlinked], &[model_id], None)
        .await
        .unwrap();
    label_db::remove_all_labels_from_models(&db, &user, &[cleared_model_id], None)
        .await
        .unwrap();
    label_keyword_db::set_keywords_for_label(&db, &user, keyworded, vec![], None)
        .await
        .unwrap();
    label_db::set_childs_on_label(&db, &user, parent, &[])
        .await
        .unwrap();
    let after = time_now();

    assert_fresh_timestamp(&added.last_modified, &before, &after);
    for label_id in [
        added.id, edited, linked, unlinked, cleared, keyworded, parent,
    ] {
        assert_fresh_timestamp(&label_last_modified(&db, label_id).await, &before, &after);
    }
}

#[tokio::test]
async fn group_and_resource_mutations_without_update_timestamp_write_the_current_time() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let stale = timestamp(2000);
    let model_id = add_model(&db, &user, "model").await;
    let mut group_ids = Vec::new();
    for name in ["edited", "source", "target"] {
        group_ids.push(
            group_db::add_empty_group(&db, &user, name, Some(&stale))
                .await
                .unwrap()
                .id,
        );
    }
    let [edited_group, source_group, target_group]: [i64; 3] = group_ids.try_into().unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(source_group), vec![model_id], Some(&stale))
        .await
        .unwrap();
    let mut resource_ids = Vec::new();
    for name in ["edited", "old", "new"] {
        resource_ids.push(
            resource_db::add_resource(&db, &user, name, Some(&stale))
                .await
                .unwrap()
                .id,
        );
    }
    let [edited_resource, old_resource, new_resource]: [i64; 3] = resource_ids.try_into().unwrap();
    resource_db::set_resource_on_group(&db, &user, Some(old_resource), source_group, Some(&stale))
        .await
        .unwrap();

    let before = time_now();
    let added_group = group_db::add_empty_group(&db, &user, "added", None)
        .await
        .unwrap();
    group_db::edit_group(&db, &user, edited_group, "edited", None, None)
        .await
        .unwrap();
    let added_resource = resource_db::add_resource(&db, &user, "added", None)
        .await
        .unwrap();
    resource_db::edit_resource(
        &db,
        &user,
        edited_resource,
        "edited",
        ResourceFlags::empty(),
        None,
        None,
    )
    .await
    .unwrap();
    resource_db::set_resource_on_group(&db, &user, Some(new_resource), source_group, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(target_group), vec![model_id], None)
        .await
        .unwrap();
    let after = time_now();

    assert_fresh_timestamp(&added_group.last_modified, &before, &after);
    assert_fresh_timestamp(&added_resource.last_modified, &before, &after);
    for group_id in [added_group.id, edited_group, source_group, target_group] {
        assert_fresh_timestamp(&group_last_modified(&db, group_id).await, &before, &after);
    }
    for resource_id in [
        added_resource.id,
        edited_resource,
        old_resource,
        new_resource,
    ] {
        assert_fresh_timestamp(
            &resource_last_modified(&db, resource_id).await,
            &before,
            &after,
        );
    }
}

// The foreign edit is allowed to either no-op or report RowNotFound; what must hold is that
// the other user's row is left untouched.
#[tokio::test]
async fn edit_model_global_id_persists_valid_rejects_invalid_and_skips_foreign_rows() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let model_id = add_model(&db, &user, "own").await;
    let foreign_id = add_model(&db, &other_user, "foreign").await;
    let foreign_global_id = model_db::get_model_via_id(&db, &other_user, foreign_id)
        .await
        .unwrap()
        .unwrap()
        .unique_global_id;

    model_db::edit_model(
        &db,
        &user,
        model_id,
        "own",
        None,
        None,
        ModelFlags::empty(),
        None,
        Some(NEW_GLOBAL_ID),
    )
    .await
    .unwrap();
    for invalid in INVALID_GLOBAL_IDS {
        let result = model_db::edit_model(
            &db,
            &user,
            model_id,
            "partial",
            None,
            None,
            ModelFlags::empty(),
            None,
            Some(invalid),
        )
        .await;
        assert!(
            matches!(result, Err(sqlx::Error::InvalidArgument(_))),
            "{invalid:?}"
        );
    }
    let foreign_result = model_db::edit_model(
        &db,
        &user,
        foreign_id,
        "hijacked",
        None,
        None,
        ModelFlags::empty(),
        None,
        Some(NEW_GLOBAL_ID),
    )
    .await;

    assert!(matches!(
        foreign_result,
        Ok(()) | Err(sqlx::Error::RowNotFound)
    ));
    let own = model_db::get_model_via_id(&db, &user, model_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (own.name.as_str(), own.unique_global_id.as_str()),
        ("own", NEW_GLOBAL_ID)
    );
    let foreign = model_db::get_model_via_id(&db, &other_user, foreign_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (foreign.name, foreign.unique_global_id),
        ("foreign".to_string(), foreign_global_id)
    );
}

#[tokio::test]
async fn edit_label_global_id_persists_valid_rejects_invalid_and_skips_foreign_rows() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let label_id = label_db::add_label(&db, &user, "own", 1, None)
        .await
        .unwrap()
        .id;
    let foreign = label_db::add_label(&db, &other_user, "foreign", 2, None)
        .await
        .unwrap();

    label_db::edit_label(&db, &user, label_id, "own", 1, None, Some(NEW_GLOBAL_ID))
        .await
        .unwrap();
    for invalid in INVALID_GLOBAL_IDS {
        let result =
            label_db::edit_label(&db, &user, label_id, "partial", 1, None, Some(invalid)).await;
        assert!(
            matches!(result, Err(sqlx::Error::InvalidArgument(_))),
            "{invalid:?}"
        );
    }
    let foreign_result = label_db::edit_label(
        &db,
        &user,
        foreign.id,
        "hijacked",
        3,
        None,
        Some(NEW_GLOBAL_ID),
    )
    .await;

    assert!(matches!(
        foreign_result,
        Ok(()) | Err(sqlx::Error::RowNotFound)
    ));
    let own_labels = label_db::get_labels_min(&db, &user).await.unwrap();
    assert_eq!(
        (
            own_labels[0].name.as_str(),
            own_labels[0].unique_global_id.as_str()
        ),
        ("own", NEW_GLOBAL_ID)
    );
    let foreign_labels = label_db::get_labels_min(&db, &other_user).await.unwrap();
    assert_eq!(foreign_labels.len(), 1);
    assert_eq!(
        (
            foreign_labels[0].name.as_str(),
            foreign_labels[0].color,
            foreign_labels[0].unique_global_id.as_str(),
        ),
        ("foreign", 2, foreign.unique_global_id.as_str())
    );
}

#[tokio::test]
async fn edit_resource_global_id_persists_valid_rejects_invalid_and_skips_foreign_rows() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let resource_id = resource_db::add_resource(&db, &user, "own", None)
        .await
        .unwrap()
        .id;
    let foreign = resource_db::add_resource(&db, &other_user, "foreign", None)
        .await
        .unwrap();

    resource_db::edit_resource(
        &db,
        &user,
        resource_id,
        "own",
        ResourceFlags::empty(),
        None,
        Some(NEW_GLOBAL_ID),
    )
    .await
    .unwrap();
    for invalid in INVALID_GLOBAL_IDS {
        let result = resource_db::edit_resource(
            &db,
            &user,
            resource_id,
            "partial",
            ResourceFlags::empty(),
            None,
            Some(invalid),
        )
        .await;
        assert!(
            matches!(result, Err(sqlx::Error::InvalidArgument(_))),
            "{invalid:?}"
        );
    }
    let foreign_result = resource_db::edit_resource(
        &db,
        &user,
        foreign.id,
        "hijacked",
        ResourceFlags::Completed,
        None,
        Some(NEW_GLOBAL_ID),
    )
    .await;

    assert!(matches!(
        foreign_result,
        Ok(()) | Err(sqlx::Error::RowNotFound)
    ));
    let own = resource_db::get_resource_meta_by_id(&db, &user, resource_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (own.name.as_str(), own.unique_global_id.as_str()),
        ("own", NEW_GLOBAL_ID)
    );
    let foreign_after = resource_db::get_resource_meta_by_id(&db, &other_user, foreign.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resource_fields(&foreign_after), resource_fields(&foreign));
}

#[tokio::test]
async fn edit_group_global_id_persists_valid_rejects_invalid_and_skips_foreign_rows() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let group_id = group_db::add_empty_group(&db, &user, "own", None)
        .await
        .unwrap()
        .id;
    let foreign = group_db::add_empty_group(&db, &other_user, "foreign", None)
        .await
        .unwrap();
    let group_name_and_global_id = async |group_id: i64| -> (String, String) {
        sqlx::query_as(
            "SELECT group_name, group_unique_global_id FROM models_group WHERE group_id = ?",
        )
        .bind(group_id)
        .fetch_one(&db)
        .await
        .unwrap()
    };

    group_db::edit_group(&db, &user, group_id, "own", None, Some(NEW_GLOBAL_ID))
        .await
        .unwrap();
    for invalid in INVALID_GLOBAL_IDS {
        let result =
            group_db::edit_group(&db, &user, group_id, "partial", None, Some(invalid)).await;
        assert!(
            matches!(result, Err(sqlx::Error::InvalidArgument(_))),
            "{invalid:?}"
        );
    }
    let foreign_result = group_db::edit_group(
        &db,
        &user,
        foreign.id,
        "hijacked",
        None,
        Some(NEW_GLOBAL_ID),
    )
    .await;

    assert_row_not_found(&foreign_result, "foreign group id");
    assert_eq!(
        group_name_and_global_id(group_id).await,
        ("own".to_string(), NEW_GLOBAL_ID.to_string())
    );
    assert_eq!(
        group_name_and_global_id(foreign.id).await,
        ("foreign".to_string(), foreign.unique_global_id.clone())
    );
}

#[tokio::test]
async fn edit_group_fails_on_an_unknown_id_and_succeeds_with_unchanged_values() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let group_id = group_db::add_empty_group(&db, &user, "same", Some(&timestamp(2000)))
        .await
        .unwrap()
        .id;

    let unknown_result = group_db::edit_group(&db, &user, MISSING_ID, "renamed", None, None).await;
    // SQLite counts a matched row as changed even when every value stays the same.
    let unchanged_result =
        group_db::edit_group(&db, &user, group_id, "same", Some(&timestamp(2000)), None).await;

    assert_row_not_found(&unknown_result, "unknown group id");
    assert!(unchanged_result.is_ok(), "{unchanged_result:?}");
    assert_eq!(group_last_modified(&db, group_id).await, timestamp(2000));
}

/// Reads `labels_labels` directly, so assertions don't depend on `label_db::get_labels`.
async fn parent_ids_of(db: &DbContext, label_id: i64) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT parent_label_id FROM labels_labels WHERE child_label_id = ? ORDER BY parent_label_id",
    )
    .bind(label_id)
    .fetch_all(db)
    .await
    .unwrap()
}

/// Inserts the edge raw, so the parent keeps its `last_modified`.
async fn link_labels(db: &DbContext, parent_label_id: i64, child_label_id: i64) {
    sqlx::query("INSERT INTO labels_labels (parent_label_id, child_label_id) VALUES (?, ?)")
        .bind(parent_label_id)
        .bind(child_label_id)
        .execute(db)
        .await
        .unwrap();
}

async fn add_label_from_2000(db: &DbContext, user: &User, name: &str) -> i64 {
    label_db::add_label(db, user, name, 0, Some(&timestamp(2000)))
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn set_label_parent_moves_the_label_and_stamps_it_and_both_parents() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let old_parent_id = add_label_from_2000(&db, &user, "old-parent").await;
    let new_parent_id = add_label_from_2000(&db, &user, "new-parent").await;
    let label_id = add_label_from_2000(&db, &user, "label").await;
    let sibling_id = add_label_from_2000(&db, &user, "sibling").await;
    let bystander_id = add_label_from_2000(&db, &user, "bystander").await;
    link_labels(&db, old_parent_id, label_id).await;
    link_labels(&db, old_parent_id, sibling_id).await;

    label_db::set_label_parent(
        &db,
        &user,
        label_id,
        Some(new_parent_id),
        Some(&timestamp(2001)),
    )
    .await
    .unwrap();

    assert_eq!(parent_ids_of(&db, label_id).await, vec![new_parent_id]);
    assert_eq!(parent_ids_of(&db, sibling_id).await, vec![old_parent_id]);
    for stamped_id in [label_id, old_parent_id, new_parent_id] {
        assert_eq!(
            label_last_modified(&db, stamped_id).await,
            timestamp(2001),
            "label {stamped_id}"
        );
    }
    for untouched_id in [sibling_id, bystander_id] {
        assert_eq!(
            label_last_modified(&db, untouched_id).await,
            timestamp(2000),
            "label {untouched_id}"
        );
    }
}

#[tokio::test]
async fn set_label_parent_with_none_makes_the_label_top_level_and_stamps_the_current_time() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let parent_id = add_label_from_2000(&db, &user, "parent").await;
    let label_id = add_label_from_2000(&db, &user, "label").await;
    link_labels(&db, parent_id, label_id).await;

    let before = time_now();
    label_db::set_label_parent(&db, &user, label_id, None, None)
        .await
        .unwrap();
    let after = time_now();

    assert_eq!(parent_ids_of(&db, label_id).await, Vec::<i64>::new());
    assert_fresh_timestamp(&label_last_modified(&db, label_id).await, &before, &after);
    assert_fresh_timestamp(&label_last_modified(&db, parent_id).await, &before, &after);
}

#[tokio::test]
async fn set_label_parent_refuses_a_foreign_or_unknown_label_or_parent_and_changes_nothing() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let parent_id = add_label_from_2000(&db, &user, "parent").await;
    let label_id = add_label_from_2000(&db, &user, "label").await;
    let foreign_id = add_label_from_2000(&db, &other_user, "foreign").await;
    link_labels(&db, parent_id, label_id).await;

    for (child_id, new_parent_id) in [
        (foreign_id, Some(parent_id)),
        (label_id, Some(foreign_id)),
        (label_id, Some(MISSING_ID)),
        (MISSING_ID, None),
    ] {
        let result =
            label_db::set_label_parent(&db, &user, child_id, new_parent_id, Some(&timestamp(2001)))
                .await;
        assert_row_not_found(
            &result,
            &format!("label {child_id} under {new_parent_id:?}"),
        );
    }

    assert_eq!(parent_ids_of(&db, label_id).await, vec![parent_id]);
    assert_eq!(parent_ids_of(&db, foreign_id).await, Vec::<i64>::new());
    for checked_id in [parent_id, label_id, foreign_id] {
        assert_eq!(
            label_last_modified(&db, checked_id).await,
            timestamp(2000),
            "label {checked_id}"
        );
    }
}

async fn keyword_names(db: &DbContext, user: &User, label_id: i64) -> Vec<String> {
    let mut names: Vec<String> = label_keyword_db::get_keywords_for_label(db, user, label_id)
        .await
        .unwrap()
        .into_iter()
        .map(|keyword| keyword.name)
        .collect();
    names.sort_unstable();

    names
}

fn keywords(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

#[tokio::test]
async fn set_keywords_for_label_keeps_the_old_keywords_when_another_label_holds_one() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let label_id = add_label_from_2000(&db, &user, "boats").await;
    let foreign_label_id = add_label_from_2000(&db, &other_user, "rafts").await;
    label_keyword_db::set_keywords_for_label(
        &db,
        &user,
        label_id,
        keywords(&["boat", "ship"]),
        Some(&timestamp(2001)),
    )
    .await
    .unwrap();
    label_keyword_db::set_keywords_for_label(
        &db,
        &other_user,
        foreign_label_id,
        keywords(&["raft"]),
        None,
    )
    .await
    .unwrap();

    // Keyword names are unique across every user's labels.
    let result = label_keyword_db::set_keywords_for_label(
        &db,
        &user,
        label_id,
        keywords(&["hull", "raft"]),
        Some(&timestamp(2002)),
    )
    .await;

    assert!(
        matches!(&result, Err(sqlx::Error::Database(error)) if error.is_unique_violation()),
        "{result:?}"
    );
    assert_eq!(keyword_names(&db, &user, label_id).await, ["boat", "ship"]);
    assert_eq!(label_last_modified(&db, label_id).await, timestamp(2001));
    assert_eq!(
        keyword_names(&db, &other_user, foreign_label_id).await,
        ["raft"]
    );
}

#[tokio::test]
async fn set_keywords_for_label_drops_duplicate_keywords() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let label_id = add_label_from_2000(&db, &user, "boats").await;

    label_keyword_db::set_keywords_for_label(
        &db,
        &user,
        label_id,
        keywords(&["hull", "keel", "hull"]),
        None,
    )
    .await
    .unwrap();

    assert_eq!(keyword_names(&db, &user, label_id).await, ["hull", "keel"]);
}

#[tokio::test]
async fn label_ownership_checks_reject_missing_foreign_and_duplicate_label_ids() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let model_id = add_model(&db, &user, "model").await;
    let own = label_db::add_label(&db, &user, "own", 0, None)
        .await
        .unwrap();
    let child_id = label_db::add_label(&db, &user, "child", 0, None)
        .await
        .unwrap()
        .id;
    let foreign_id = label_db::add_label(&db, &other_user, "foreign", 0, None)
        .await
        .unwrap()
        .id;

    for label_ids in [
        vec![MISSING_ID],
        vec![foreign_id],
        vec![own.id, foreign_id],
        vec![own.id, own.id],
    ] {
        let context = format!("label ids {label_ids:?}");
        assert_row_not_found(
            &label_db::add_labels_on_models(&db, &user, &label_ids, &[model_id], None).await,
            &context,
        );
        assert_row_not_found(
            &label_db::remove_labels_from_models(&db, &user, &label_ids, &[model_id], None).await,
            &context,
        );
    }
    for child_ids in [vec![MISSING_ID], vec![foreign_id], vec![child_id, child_id]] {
        assert_row_not_found(
            &label_db::set_childs_on_label(&db, &user, own.id, &child_ids).await,
            &format!("child ids {child_ids:?}"),
        );
    }
    for parent_id in [MISSING_ID, foreign_id] {
        let context = format!("parent id {parent_id}");
        assert_row_not_found(
            &label_db::set_childs_on_label(&db, &user, parent_id, &[]).await,
            &context,
        );
        assert_row_not_found(
            &label_keyword_db::set_keywords_for_label(
                &db,
                &user,
                parent_id,
                vec!["keyword".to_string()],
                None,
            )
            .await,
            &context,
        );
    }
    label_db::add_labels_on_models(&db, &user, &[], &[model_id], None)
        .await
        .unwrap();

    let model = model_db::get_model_via_id(&db, &user, model_id)
        .await
        .unwrap()
        .unwrap();
    assert!(model.labels.is_empty());
    assert!(
        label_keyword_db::get_all_keywords(&db, &other_user)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn model_ownership_checks_reject_missing_foreign_and_duplicate_model_ids() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let model_id = add_model(&db, &user, "own").await;
    let foreign_id = add_model(&db, &other_user, "foreign").await;
    let label_id = label_db::add_label(&db, &user, "label", 0, None)
        .await
        .unwrap()
        .id;
    label_db::add_labels_on_models(&db, &user, &[label_id], &[model_id], None)
        .await
        .unwrap();
    let share_id = share_db::create_share(&db, &user, "share").await.unwrap();

    for model_ids in [
        vec![MISSING_ID],
        vec![foreign_id],
        vec![model_id, foreign_id],
        vec![model_id, model_id],
    ] {
        let context = format!("model ids {model_ids:?}");
        assert_row_not_found(
            &label_db::remove_labels_from_models(&db, &user, &[label_id], &model_ids, None).await,
            &context,
        );
        assert_row_not_found(
            &label_db::remove_all_labels_from_models(&db, &user, &model_ids, None).await,
            &context,
        );
        assert_row_not_found(
            &share_db::set_model_ids_on_share(&db, &user, &share_id, model_ids.clone()).await,
            &context,
        );
    }

    let model = model_db::get_model_via_id(&db, &user, model_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        model
            .labels
            .iter()
            .map(|label| label.id)
            .collect::<Vec<_>>(),
        vec![label_id]
    );
    assert_eq!(
        share_db::get_share_via_id(&db, &share_id)
            .await
            .unwrap()
            .model_ids,
        Vec::<i64>::new()
    );
}

#[tokio::test]
async fn resource_and_group_ownership_checks_reject_missing_foreign_and_empty_targets() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let resource = resource_db::add_resource(&db, &user, "own", None)
        .await
        .unwrap();
    let foreign_resource_id = resource_db::add_resource(&db, &other_user, "foreign", None)
        .await
        .unwrap()
        .id;
    let group_id = add_group_with_model(&db, &user, "own").await;
    let foreign_group_id = add_group_with_model(&db, &other_user, "foreign").await;
    let empty_group_id = group_db::add_empty_group(&db, &user, "empty", None)
        .await
        .unwrap()
        .id;
    let model_id = add_model(&db, &user, "ungrouped").await;

    for (resource_id, target_group_id) in [
        (foreign_resource_id, group_id),
        (MISSING_ID, group_id),
        (resource.id, foreign_group_id),
        (resource.id, MISSING_ID),
        (resource.id, empty_group_id),
    ] {
        assert_row_not_found(
            &resource_db::set_resource_on_group(
                &db,
                &user,
                Some(resource_id),
                target_group_id,
                None,
            )
            .await,
            &format!("resource {resource_id} on group {target_group_id}"),
        );
    }
    assert_row_not_found(
        &group_db::set_group_id_on_models(&db, &user, Some(MISSING_ID), vec![model_id], None).await,
        "missing group id",
    );

    assert!(
        resource_db::get_group_id_to_resource_map(&db, &user)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        resource_db::get_group_id_to_resource_map(&db, &other_user)
            .await
            .unwrap()
            .is_empty()
    );
    let model = model_db::get_model_via_id(&db, &user, model_id)
        .await
        .unwrap()
        .unwrap();
    assert!(model.group.is_none());
}

#[tokio::test]
async fn get_model_ids_via_sha256s_preserves_input_order_and_rejects_unknown_or_foreign() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let first = add_model(&db, &user, "first").await;
    let second = add_model(&db, &user, "second").await;
    add_model(&db, &other_user, "foreign").await;

    let reordered =
        model_db::get_model_ids_via_sha256s(&db, &user, &["second", "first"].map(String::from))
            .await
            .unwrap();
    let duplicated =
        model_db::get_model_ids_via_sha256s(&db, &user, &["first", "first"].map(String::from))
            .await
            .unwrap();
    let empty = model_db::get_model_ids_via_sha256s(&db, &user, &[])
        .await
        .unwrap();

    assert_eq!(reordered, vec![second, first]);
    assert_eq!(duplicated, vec![first, first]);
    assert_eq!(empty, Vec::<i64>::new());
    for sha256s in [["first", "missing"], ["first", "foreign"]] {
        assert_row_not_found(
            &model_db::get_model_ids_via_sha256s(&db, &user, &sha256s.map(String::from)).await,
            &format!("sha256s {sha256s:?}"),
        );
    }
    assert_eq!(
        model_db::get_model_id_via_sha256(&db, &user, "first")
            .await
            .unwrap(),
        Some(first)
    );
    assert_eq!(
        model_db::get_model_id_via_sha256(&db, &user, "foreign")
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn set_group_id_on_models_rejects_another_users_group() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let foreign_group_id = add_group_with_model(&db, &other_user, "theirs").await;
    let model_id = add_model(&db, &user, "mine").await;

    let result =
        group_db::set_group_id_on_models(&db, &user, Some(foreign_group_id), vec![model_id], None)
            .await;

    assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    let models = model_db::get_models_via_ids(&db, &user, vec![model_id])
        .await
        .unwrap();
    assert!(models[0].group.is_none());
}

#[tokio::test]
async fn add_labels_on_models_rejects_another_users_model() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let label_id = label_db::add_label(&db, &user, "mine", 1, None)
        .await
        .unwrap()
        .id;
    let foreign_model_id = add_model(&db, &other_user, "theirs").await;

    let result =
        label_db::add_labels_on_models(&db, &user, &[label_id], &[foreign_model_id], None).await;

    assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    let models = model_db::get_models_via_ids(&db, &other_user, vec![foreign_model_id])
        .await
        .unwrap();
    assert!(models[0].labels.is_empty());
}

#[tokio::test]
async fn set_model_ids_on_share_with_no_models_clears_the_share() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let model_id = add_model(&db, &user, "shared").await;
    let share_id = share_db::create_share(&db, &user, "share").await.unwrap();
    share_db::set_model_ids_on_share(&db, &user, &share_id, vec![model_id])
        .await
        .unwrap();

    share_db::set_model_ids_on_share(&db, &user, &share_id, vec![])
        .await
        .unwrap();

    let share = share_db::get_share_via_id(&db, &share_id).await.unwrap();
    assert_eq!(share.model_ids, Vec::<i64>::new());
}

#[tokio::test]
async fn set_labels_on_model_rejects_another_users_label_without_dropping_existing_labels() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let model_id = add_model(&db, &user, "mine").await;
    let kept_label_id = label_db::add_label(&db, &user, "kept", 1, None)
        .await
        .unwrap()
        .id;
    label_db::add_labels_on_models(&db, &user, &[kept_label_id], &[model_id], None)
        .await
        .unwrap();
    let foreign_label_id = label_db::add_label(&db, &other_user, "theirs", 1, None)
        .await
        .unwrap()
        .id;

    let result = label_db::set_labels_on_model(&db, &user, &[foreign_label_id], model_id).await;

    assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    let models = model_db::get_models_via_ids(&db, &user, vec![model_id])
        .await
        .unwrap();
    let label_ids: Vec<i64> = models[0].labels.iter().map(|label| label.id).collect();
    assert_eq!(label_ids, vec![kept_label_id]);
}

async fn count_rows(db: &DbContext, sql: &'static str, id: i64) -> i64 {
    sqlx::query_scalar::<_, i64>(sql)
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
}

/// Pins that `PRAGMA foreign_keys` is on for the pool, so `ON DELETE CASCADE` fires.
#[tokio::test]
async fn delete_user_cascades_to_every_owned_row() {
    let (_dir, db) = test_db().await;
    let user = User::default();
    let other_user = add_other_user(&db).await;
    let control_model_id = add_model(&db, &user, "control").await;
    let control_label_id = label_db::add_label(&db, &user, "control", 0, None)
        .await
        .unwrap()
        .id;
    let control_group_id = group_db::add_empty_group(&db, &user, "control", None)
        .await
        .unwrap()
        .id;
    let control_resource_id = resource_db::add_resource(&db, &user, "control", None)
        .await
        .unwrap()
        .id;
    let control_share_id = share_db::create_share(&db, &user, "control").await.unwrap();
    let model_id = add_model(&db, &other_user, "theirs").await;
    let label_id = label_db::add_label(&db, &other_user, "theirs", 1, None)
        .await
        .unwrap()
        .id;
    let child_label_id = label_db::add_label(&db, &other_user, "child", 1, None)
        .await
        .unwrap()
        .id;
    label_db::set_childs_on_label(&db, &other_user, label_id, &[child_label_id])
        .await
        .unwrap();
    label_db::add_labels_on_models(&db, &other_user, &[label_id], &[model_id], None)
        .await
        .unwrap();
    label_keyword_db::set_keywords_for_label(
        &db,
        &other_user,
        label_id,
        vec!["keyword".to_string()],
        None,
    )
    .await
    .unwrap();
    add_group_with_model(&db, &other_user, "group").await;
    resource_db::add_resource(&db, &other_user, "resource", None)
        .await
        .unwrap();
    let share_id = share_db::create_share(&db, &other_user, "share")
        .await
        .unwrap();
    share_db::set_model_ids_on_share(&db, &other_user, &share_id, vec![model_id])
        .await
        .unwrap();

    let link_queries = [
        "SELECT COUNT(*) FROM models_labels",
        "SELECT COUNT(*) FROM labels_labels",
        "SELECT COUNT(*) FROM shares_models",
        "SELECT COUNT(*) FROM label_keywords",
    ];
    for sql in link_queries {
        let count: i64 = sqlx::query_scalar(sql).fetch_one(&db).await.unwrap();
        assert_eq!(count, 1, "{sql}");
    }

    user_db::delete_user(&db, other_user.id).await.unwrap();

    for sql in [
        "SELECT COUNT(*) FROM models WHERE model_user_id = ?",
        "SELECT COUNT(*) FROM labels WHERE label_user_id = ?",
        "SELECT COUNT(*) FROM models_group WHERE group_user_id = ?",
        "SELECT COUNT(*) FROM resources WHERE resource_user_id = ?",
        "SELECT COUNT(*) FROM shares WHERE share_user_id = ?",
    ] {
        assert_eq!(count_rows(&db, sql, other_user.id).await, 0, "{sql}");
    }
    for sql in link_queries {
        let count: i64 = sqlx::query_scalar(sql).fetch_one(&db).await.unwrap();
        assert_eq!(count, 0, "{sql}");
    }
    for (sql, id) in [
        (
            "SELECT COUNT(*) FROM models WHERE model_id = ?",
            control_model_id,
        ),
        (
            "SELECT COUNT(*) FROM labels WHERE label_id = ?",
            control_label_id,
        ),
        (
            "SELECT COUNT(*) FROM models_group WHERE group_id = ?",
            control_group_id,
        ),
        (
            "SELECT COUNT(*) FROM resources WHERE resource_id = ?",
            control_resource_id,
        ),
    ] {
        assert_eq!(count_rows(&db, sql, id).await, 1, "{sql}");
    }
    let control_share = share_db::get_share_via_id(&db, &control_share_id)
        .await
        .unwrap();
    assert_eq!(control_share.user_id, user.id);
}
