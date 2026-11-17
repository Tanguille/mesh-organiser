//! A legacy Keep request did not state whether the project should be cleared.

use std::fmt::Write;

use db::group_db;

mod common;

fn encode(payload: &mut String, value: Option<&str>) {
    if let Some(value) = value {
        write!(payload, "{}:{value}", value.len()).unwrap();
    } else {
        payload.push('-');
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn legacy_keep_journal_cannot_claim_explicit_none_as_an_exact_replay() {
    let (_dir, db) = common::test_db().await;
    let user = db::model::user::User::default();
    let project = db::resource_db::add_resource(&db, &user, "project", None)
        .await
        .unwrap();
    let group = group_db::add_empty_group(&db, &user, "folder", None)
        .await
        .unwrap();
    sqlx::query("UPDATE models_group SET group_resource_id = ? WHERE group_id = ?")
        .bind(project.id)
        .bind(group.id)
        .execute(&db)
        .await
        .unwrap();
    let birth: String =
        sqlx::query_scalar("SELECT group_birth_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let snapshot = group_db::get_group_sync_snapshot(&db, &user).await.unwrap();
    let info = snapshot
        .groups
        .iter()
        .find(|item| item.global_id == group.unique_global_id)
        .unwrap();
    let head = snapshot
        .heads
        .iter()
        .find(|head| head.group_global_id == group.unique_global_id)
        .unwrap();
    let deletion = snapshot
        .deletions
        .iter()
        .find(|deletion| deletion.group_global_id == group.unique_global_id)
        .unwrap()
        .operation_id
        .clone();
    let condition = group_db::GroupPrecondition {
        group_global_id: group.unique_global_id.clone(),
        expected_generation: head.generation,
        expected_head: head.head,
        expected_state: head.state.clone(),
        expected_revision: Some(info.revision),
        expected_deleted_at: info.deleted_at.clone(),
    };
    let action = group_db::GroupLifecycleChange::Keep {
        group_global_id: group.unique_global_id.clone(),
        deletion_operation_id: deletion.clone(),
        expected_birth_id: Some(birth.clone()),
        resource: group_db::KeepResource::None,
        name: "folder".into(),
    };
    // Payload format used before Keep requests explicitly specified project intent.
    let mut legacy = String::from("g");
    encode(&mut legacy, Some(&condition.group_global_id));
    encode(
        &mut legacy,
        Some(&condition.expected_generation.to_string()),
    );
    encode(&mut legacy, Some(&condition.expected_head.to_string()));
    encode(&mut legacy, Some(&condition.expected_state));
    encode(
        &mut legacy,
        condition
            .expected_revision
            .map(|expected_revision| expected_revision.to_string())
            .as_deref(),
    );
    encode(&mut legacy, condition.expected_deleted_at.as_deref());
    legacy.push('l');
    encode(&mut legacy, Some("30"));
    legacy.push('k');
    encode(&mut legacy, Some(&group.unique_global_id));
    encode(&mut legacy, Some(&deletion));
    encode(&mut legacy, Some("folder"));
    legacy.push('b');
    encode(&mut legacy, Some(&birth));
    let operation = db::random_hex_32();
    sqlx::query(
        "INSERT INTO assignment_batch_journal (user_id, operation_id, payload) VALUES (?, ?, ?)",
    )
    .bind(user.id)
    .bind(&operation)
    .bind(legacy)
    .execute(&db)
    .await
    .unwrap();
    let result =
        group_db::apply_group_sync_batch(&db, &user, &operation, &[], &[condition], &[action], 30)
            .await;
    assert!(
        result.is_err(),
        "an old request with unspecified project intent cannot be an exact replay"
    );
    let resource: Option<i64> =
        sqlx::query_scalar("SELECT group_resource_id FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(resource, Some(project.id));
}
