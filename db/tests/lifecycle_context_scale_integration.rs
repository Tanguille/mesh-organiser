//! Unrelated folders must not consume one operation's causal context limit.
use db::{causal_group_lifecycle, group_db};

mod common;

#[tokio::test]
async fn empty_group_deletion_ignores_over_1024_unrelated_scopes() {
    let (_dir, db) = common::test_db().await;
    let user = db::model::user::User::default();
    let identity = db::user_db::get_sync_identity(&db, user.id)
        .await
        .unwrap()
        .unwrap();
    let target = group_db::add_empty_group(&db, &user, "target", None)
        .await
        .unwrap();
    causal_group_lifecycle::enroll_pristine_group(&db, &user, &identity, target.id)
        .await
        .unwrap();
    for index in 0..1025 {
        let group = group_db::add_empty_group(&db, &user, &format!("unrelated-{index}"), None)
            .await
            .unwrap();
        causal_group_lifecycle::enroll_pristine_group(&db, &user, &identity, group.id)
            .await
            .unwrap();
    }
    group_db::delete_group(&db, &user, target.id)
        .await
        .expect("unrelated scopes must not prevent deleting an empty group");
    let state = causal_group_lifecycle::get_state(&db, &user, &identity, target.id)
        .await
        .unwrap();
    let causal_group_lifecycle::GroupLifecycle::Deleted { deletions } = state.lifecycle else {
        panic!("expected a portable deletion")
    };
    let operation = causal_group_lifecycle::export_operation(&db, &user, &identity, &deletions[0])
        .await
        .unwrap();
    assert_eq!(operation.envelopes().len(), 1);
    let encoded = db::sync_transport::encode_operation(&operation).unwrap();
    let context: serde_json::Value =
        serde_json::from_str(encoded["author_context"].as_str().unwrap()).unwrap();
    let frontier = context["frontier"].as_array().unwrap();
    assert_eq!(
        frontier.len(),
        1,
        "only the target group's frontier belongs to this cut"
    );
    assert_eq!(frontier[0]["entity"]["global_id"], target.unique_global_id);
}
