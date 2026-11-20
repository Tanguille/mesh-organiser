//! A group cannot receive a model after a concurrent deletion has committed.

use db::{group_db, model::user::User};

mod common;

const ITERATIONS: usize = 20;

#[tokio::test(flavor = "multi_thread")]
async fn assigning_a_model_racing_group_deletion_never_hides_the_model_in_trash() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let source = group_db::add_empty_group(&db, &user, "source", None)
        .await
        .unwrap();
    let blob = db::blob_db::add_blob(&db, "race-model", "stl", 1, None)
        .await
        .unwrap();
    let model = db::model_db::add_model(&db, &user, "model", blob, None, None)
        .await
        .unwrap();

    for iteration in 0..ITERATIONS {
        // Reset the model into the source group, with a fresh destination to race for.
        group_db::set_group_id_on_models(&db, &user, Some(source.id), vec![model], None)
            .await
            .unwrap();
        let destination =
            group_db::add_empty_group(&db, &user, &format!("destination {iteration}"), None)
                .await
                .unwrap();

        // Separate tasks on the multi-thread runtime, so either side can win the write lock.
        let assignment = tokio::spawn({
            let db = db.clone();
            let user = user.clone();
            async move {
                group_db::set_group_id_on_models(
                    &db,
                    &user,
                    Some(destination.id),
                    vec![model],
                    None,
                )
                .await
            }
        });
        let deletion = tokio::spawn({
            let db = db.clone();
            let user = user.clone();
            async move { group_db::delete_group(&db, &user, destination.id).await }
        });
        let assignment = assignment.await.unwrap();
        deletion.await.unwrap().unwrap();

        let current: Option<i64> =
            sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
                .bind(model)
                .fetch_one(&db)
                .await
                .unwrap();
        if assignment.is_ok() {
            // The assignment committed first, so the deletion must have detached the model.
            assert_eq!(
                current, None,
                "iteration {iteration}: model left in a deleted group"
            );
        } else {
            // The deletion committed first, so the move was rejected and the source is kept.
            assert_eq!(
                current,
                Some(source.id),
                "iteration {iteration}: a rejected move must preserve its source"
            );
        }
    }
}
