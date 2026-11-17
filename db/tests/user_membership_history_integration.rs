//! User membership changes and group addition events share causal frontiers.
mod common;

use db::{
    blob_db,
    db_context::DbContext,
    group_db,
    group_history::{self, GroupLifecycleAction, HistoryOrigin, MembershipAction},
    model::user::User,
    model_db, random_hex_32, user_db,
};

async fn add_model(db: &DbContext, user: &User, name: &str) -> (i64, String, String) {
    let blob = blob_db::add_blob(db, name, "stl", 1, None).await.unwrap();
    let id = model_db::add_model(db, user, name, blob, None, None)
        .await
        .unwrap();
    let (global, incarnation) = sqlx::query_as(
        "SELECT model_unique_global_id, model_incarnation FROM models WHERE model_id = ?",
    )
    .bind(id)
    .fetch_one(db)
    .await
    .unwrap();
    (id, global, incarnation)
}

async fn moves(
    db: &DbContext,
    user: &User,
    global: &str,
    incarnation: &str,
) -> group_history::MembershipHistory {
    group_history::get_membership_history(db, user, global, incarnation)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn moves_record_model_predecessors_and_cross_entity_group_additions() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let a = group_db::add_empty_group(&db, &user, "A", None)
        .await
        .unwrap();
    let b = group_db::add_empty_group(&db, &user, "B", None)
        .await
        .unwrap();
    let (id, global, incarnation) = add_model(&db, &user, "model").await;
    let initial = moves(&db, &user, &global, &incarnation).await;
    assert!(matches!(initial.ancestry, HistoryOrigin::UnknownPreUpgrade));
    assert_eq!(initial.events, vec![]);
    group_db::set_group_id_on_models(&db, &user, Some(a.id), vec![id, id], None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(a.id), vec![id], None)
        .await
        .unwrap();
    let first = moves(&db, &user, &global, &incarnation).await;
    assert_eq!(first.events.len(), 1);
    assert_eq!(
        first.events[0].action,
        MembershipAction::Move {
            from_group_global_id: None,
            to_group_global_id: Some(a.unique_global_id.clone())
        }
    );
    assert_eq!(first.events[0].parents, vec![]);
    assert_eq!(first.frontier, vec![first.events[0].id.clone()]);
    let additions = group_history::get_group_member_added_history(&db, &user, &a.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(additions.len(), 1);
    assert_eq!(additions[0].model_global_id, global);
    assert_eq!(additions[0].model_incarnation, incarnation);
    assert_eq!(additions[0].membership_event, first.events[0].id);

    group_db::set_group_id_on_models(&db, &user, Some(b.id), vec![id], None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, None, vec![id], None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, None, vec![id], None)
        .await
        .unwrap();
    let complete = moves(&db, &user, &global, &incarnation).await;
    assert!(matches!(
        complete.ancestry,
        HistoryOrigin::UnknownPreUpgrade
    ));
    assert_eq!(complete.events.len(), 3);
    assert_eq!(
        complete.events[1].action,
        MembershipAction::Move {
            from_group_global_id: Some(a.unique_global_id.clone()),
            to_group_global_id: Some(b.unique_global_id.clone())
        }
    );
    assert_eq!(
        complete.events[1].parents,
        vec![complete.events[0].id.clone()]
    );
    assert_eq!(
        complete.events[2].action,
        MembershipAction::Move {
            from_group_global_id: Some(b.unique_global_id.clone()),
            to_group_global_id: None
        }
    );
    assert_eq!(
        complete.events[2].parents,
        vec![complete.events[1].id.clone()]
    );
    assert_eq!(complete.frontier, vec![complete.events[2].id.clone()]);
    let b_additions =
        group_history::get_group_member_added_history(&db, &user, &b.unique_global_id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(b_additions.len(), 1);
    assert_eq!(b_additions[0].membership_event, complete.events[1].id);
}

#[tokio::test]
async fn group_delete_observes_prior_add_and_rename_through_shared_frontier() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "before", None)
        .await
        .unwrap();
    let (id, global, incarnation) = add_model(&db, &user, "model").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![id], None)
        .await
        .unwrap();
    let addition =
        group_history::get_group_member_added_history(&db, &user, &group.unique_global_id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(addition.len(), 1);
    assert_eq!(
        addition[0].membership_event,
        moves(&db, &user, &global, &incarnation).await.events[0].id
    );
    group_db::edit_group(&db, &user, group.id, "after", None, None)
        .await
        .unwrap();
    let rename = group_history::get_group_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rename.events.len(), 1);
    assert_eq!(rename.events[0].parents, vec![addition[0].id.clone()]);
    group_db::delete_group(&db, &user, group.id).await.unwrap();
    let lifecycle = group_history::get_group_lifecycle_history(&db, &user, &group.unique_global_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lifecycle.len(), 1);
    assert!(matches!(
        lifecycle[0].action,
        GroupLifecycleAction::Delete { .. }
    ));
    assert_eq!(lifecycle[0].parents, vec![rename.events[0].id.clone()]);
}

#[tokio::test]
async fn failing_group_addition_event_rolls_back_entire_multi_model_move() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let group = group_db::add_empty_group(&db, &user, "destination", None)
        .await
        .unwrap();
    let (first, first_global, first_inc) = add_model(&db, &user, "first").await;
    let (second, second_global, second_inc) = add_model(&db, &user, "second").await;
    let before_group: i64 =
        sqlx::query_scalar("SELECT group_revision FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    sqlx::query("CREATE TRIGGER reject_group_add BEFORE INSERT ON causal_events WHEN NEW.encoding = 'local_history_v1' AND json_extract(NEW.envelope, '$.action.kind') = 'member_added' AND EXISTS (SELECT 1 FROM group_causal_event_facts WHERE event_kind = 'member_added') BEGIN SELECT RAISE(ABORT, 'injected second event failure'); END").execute(&db).await.unwrap();
    let error =
        group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![first, second], None)
            .await
            .unwrap_err();
    assert!(error.to_string().contains("injected second event failure"));
    for (id, global, inc) in [
        (first, first_global, first_inc),
        (second, second_global, second_inc),
    ] {
        let state: (Option<i64>, i64) = sqlx::query_as(
            "SELECT model_group_id, model_assignment_revision FROM models WHERE model_id = ?",
        )
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(state.0, None);
        assert_eq!(state.1, 0);
        assert_eq!(moves(&db, &user, &global, &inc).await.events, vec![]);
    }
    let after_group: i64 =
        sqlx::query_scalar("SELECT group_revision FROM models_group WHERE group_id = ?")
            .bind(group.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(after_group, before_group);
    assert_eq!(
        group_history::get_group_member_added_history(&db, &user, &group.unique_global_id)
            .await
            .unwrap()
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn user_membership_history_is_scoped_to_the_owner() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let other_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let other = user_db::get_user_by_id(&db, other_id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&db, &user, "private", None)
        .await
        .unwrap();
    let (id, global, inc) = add_model(&db, &user, "private model").await;
    group_db::set_group_id_on_models(&db, &user, Some(group.id), vec![id], None)
        .await
        .unwrap();
    assert_eq!(moves(&db, &user, &global, &inc).await.events.len(), 1);
    assert!(
        group_history::get_membership_history(&db, &other, &global, &inc)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        group_history::get_group_member_added_history(&db, &other, &group.unique_global_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn ambiguous_group_global_identity_rejects_move_without_events() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let first = group_db::add_empty_group(&db, &user, "first", None)
        .await
        .unwrap();
    let second = group_db::add_empty_group(&db, &user, "second", None)
        .await
        .unwrap();
    let (id, global, incarnation) = add_model(&db, &user, "ambiguous-model").await;
    sqlx::query("UPDATE models_group SET group_unique_global_id = ? WHERE group_id = ?")
        .bind(&first.unique_global_id)
        .bind(second.id)
        .execute(&db)
        .await
        .unwrap();
    assert!(
        group_db::set_group_id_on_models(&db, &user, Some(first.id), vec![id], None)
            .await
            .is_err()
    );
    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(assigned, None);
    assert_eq!(
        moves(&db, &user, &global, &incarnation).await.events,
        vec![]
    );
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM group_causal_event_facts WHERE user_id = ? AND group_global_id = ? AND event_kind = 'member_added'")
        .bind(user.id).bind(&first.unique_global_id).fetch_one(&db).await.unwrap();
    assert_eq!(events, 0);
}

#[tokio::test]
async fn ambiguous_source_identity_rejects_move_without_events() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let source = group_db::add_empty_group(&db, &user, "source", None)
        .await
        .unwrap();
    let duplicate = group_db::add_empty_group(&db, &user, "duplicate", None)
        .await
        .unwrap();
    let dest = group_db::add_empty_group(&db, &user, "destination", None)
        .await
        .unwrap();
    let (id, global, incarnation) = add_model(&db, &user, "ambiguous-source-model").await;
    sqlx::query("UPDATE models SET model_group_id = ? WHERE model_id = ?")
        .bind(source.id)
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE models_group SET group_unique_global_id = ? WHERE group_id = ?")
        .bind(&source.unique_global_id)
        .bind(duplicate.id)
        .execute(&db)
        .await
        .unwrap();
    assert!(
        group_db::set_group_id_on_models(&db, &user, Some(dest.id), vec![id], None)
            .await
            .is_err()
    );
    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(assigned, Some(source.id));
    assert_eq!(
        moves(&db, &user, &global, &incarnation).await.events,
        vec![]
    );
    assert_eq!(
        group_history::get_group_member_added_history(&db, &user, &dest.unique_global_id)
            .await
            .unwrap()
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn ambiguous_model_global_identity_rejects_move_without_events() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let dest = group_db::add_empty_group(&db, &user, "destination", None)
        .await
        .unwrap();
    let (first, global, _) = add_model(&db, &user, "first-model").await;
    let (second, _, _) = add_model(&db, &user, "second-model").await;
    sqlx::query("UPDATE models SET model_unique_global_id = ? WHERE model_id = ?")
        .bind(&global)
        .bind(second)
        .execute(&db)
        .await
        .unwrap();
    assert!(
        group_db::set_group_id_on_models(&db, &user, Some(dest.id), vec![first], None)
            .await
            .is_err()
    );
    for id in [first, second] {
        let assigned: Option<i64> =
            sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(assigned, None);
    }
    let events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM membership_causal_events WHERE user_id = ? AND model_global_id = ?",
    )
    .bind(user.id)
    .bind(global)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(events, 0);
    assert_eq!(
        group_history::get_group_member_added_history(&db, &user, &dest.unique_global_id)
            .await
            .unwrap()
            .unwrap()
            .len(),
        0
    );
}
