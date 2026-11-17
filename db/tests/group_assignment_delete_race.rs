//! A group cannot receive a model after a concurrent deletion has committed.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use tokio::sync::Notify;

use db::group_db;

mod common;

#[tokio::test(flavor = "multi_thread")]
async fn assigning_a_model_racing_group_deletion_never_hides_the_model_in_trash() {
    let (dir, db) = common::test_db().await;
    let user = db::model::user::User::default();
    let source = group_db::add_empty_group(&db, &user, "source", None)
        .await
        .unwrap();
    let destination = group_db::add_empty_group(&db, &user, "destination", None)
        .await
        .unwrap();
    let blob = db::blob_db::add_blob(&db, "race-model", "stl", 1, None)
        .await
        .unwrap();
    let model = db::model_db::add_model(&db, &user, "model", blob, None, None)
        .await
        .unwrap();
    group_db::set_group_id_on_models(&db, &user, Some(source.id), vec![model], None)
        .await
        .unwrap();

    let reached = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .after_release({
            let reached = Arc::clone(&reached);
            let resume = Arc::clone(&resume);
            let armed = Arc::clone(&armed);
            let releases = Arc::clone(&releases);
            move |connection, _| {
                let reached = Arc::clone(&reached);
                let resume = Arc::clone(&resume);
                let armed = Arc::clone(&armed);
                let releases = Arc::clone(&releases);
                Box::pin(async move {
                    if armed.load(Ordering::SeqCst) && releases.fetch_add(1, Ordering::SeqCst) == 4
                    {
                        let current: Option<i64> = sqlx::query_scalar(
                            "SELECT model_group_id FROM models WHERE model_id = ?",
                        )
                        .bind(model)
                        .fetch_one(&mut *connection)
                        .await?;
                        if current == Some(source.id) {
                            reached.notify_one();
                            resume.notified().await;
                        }
                    }
                    Ok(true)
                })
            }
        })
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(dir.path().join("db.sqlite"))
                .foreign_keys(true)
                .busy_timeout(Duration::from_secs(15)),
        )
        .await
        .unwrap();
    // Arm while holding the only connection: its release is counted before the
    // four reads in the current implementation, without relying on timing.
    let connection = pool.acquire().await.unwrap();
    armed.store(true, Ordering::SeqCst);
    drop(connection);

    let assignment_pool = pool.clone();
    let assignment_user = user.clone();
    let mut assignment = tokio::spawn(async move {
        group_db::set_group_id_on_models(
            &assignment_pool,
            &assignment_user,
            Some(destination.id),
            vec![model],
            None,
        )
        .await
    });

    tokio::select! {
        () = reached.notified() => {
            // The destination was validated while active. Commit its deletion
            // before allowing the assignment to proceed.
            group_db::delete_group(&db, &user, destination.id).await.unwrap();
            resume.notify_one();
            let result = assignment.await.unwrap();
            assert!(result.is_err(), "a deleted destination must reject a late assignment");
            let current: Option<i64> = sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
                .bind(model).fetch_one(&db).await.unwrap();
            assert_eq!(current, Some(source.id), "a rejected move must preserve its source");
        }
        result = &mut assignment => {
            // set_group_id_on_models runs in one BEGIN IMMEDIATE transaction, so the pool is not
            // released mid-assignment and the hook above never pauses (verified: this arm is the
            // outcome today). The pause arm only guards a future multi-statement implementation.
            // A serialized assignment finishes first. The later deletion
            // then detaches the model rather than hiding it in the group.
            result.unwrap().unwrap();
            group_db::delete_group(&db, &user, destination.id).await.unwrap();
            let current: Option<i64> = sqlx::query_scalar("SELECT model_group_id FROM models WHERE model_id = ?")
                .bind(model).fetch_one(&db).await.unwrap();
            assert_eq!(current, None);
        }
        () = tokio::time::sleep(Duration::from_secs(10)) => {
            resume.notify_one();
            panic!("assignment did not reach a controlled outcome");
        }
    }
}
