//! A reused model id must never let a concurrent assignment modify another owner's model.
mod common;
use db::{blob_db, group_db, model::user::User, model_db, random_hex_32, user_db};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

async fn insert_legacy_model(db: &sqlx::SqlitePool, user: &User, blob: i64, id: i64) {
    sqlx::query("INSERT INTO models (model_id, model_name, model_blob_id, model_added, model_user_id, model_unique_global_id, model_last_modified) VALUES (?, 'legacy', ?, '2020-01-01T00:00:00Z', ?, ?, '2020-01-01T00:00:00Z')")
        .bind(id).bind(blob).bind(user.id).bind(random_hex_32()).execute(db).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)]
async fn assignment_cannot_move_another_users_model_after_id_reuse() {
    let dir = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(dir.path().join("db.sqlite"))
        .create_if_missing(true)
        .foreign_keys(true);
    let db = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::migrate!("./migrations").run(&db).await.unwrap();
    // Reproduce a legacy reused id against current assignment and history code.
    sqlx::query("DROP TRIGGER model_monotonic_id")
        .execute(&db)
        .await
        .unwrap();
    let owner = User::default();
    let foreign_id = user_db::add_user(&db, "other", "other@example.test", &random_hex_32())
        .await
        .unwrap();
    let foreign = user_db::get_user_by_id(&db, foreign_id)
        .await
        .unwrap()
        .unwrap();
    let source = common::legacy_group(&db, &owner, "source").await;
    let target = common::legacy_group(&db, &owner, "target").await;
    let blob = blob_db::add_blob(&db, "reuse-race-model", "stl", 1, None)
        .await
        .unwrap();
    let model = 1;
    insert_legacy_model(&db, &owner, blob, model).await;
    group_db::set_group_id_on_models(&db, &owner, Some(source.id), vec![model], None)
        .await
        .unwrap();

    let reached = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let armed = Arc::new(AtomicBool::new(false));
    let releases = Arc::new(AtomicUsize::new(0));
    let pool = SqlitePoolOptions::new()
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
            SqliteConnectOptions::new()
                .filename(dir.path().join("db.sqlite"))
                .foreign_keys(true)
                .busy_timeout(Duration::from_secs(15)),
        )
        .await
        .unwrap();
    let connection = pool.acquire().await.unwrap();
    armed.store(true, Ordering::SeqCst);
    drop(connection);
    let assignment_pool = pool.clone();
    let assignment_user = owner.clone();
    let mut assignment = tokio::spawn(async move {
        group_db::set_group_id_on_models(
            &assignment_pool,
            &assignment_user,
            Some(target.id),
            vec![model],
            None,
        )
        .await
    });
    tokio::select! {
        () = reached.notified() => {
            model_db::trash_models(&db, &owner, &[model]).await.unwrap();
            sqlx::query("UPDATE models SET model_deleted = '2000-01-01T00:00:00Z' WHERE model_id = ?").bind(model).execute(&db).await.unwrap();
            assert_eq!(model_db::purge_trashed_models(&db, 30).await.unwrap(), 1);
            insert_legacy_model(&db, &foreign, blob, model).await;
            resume.notify_one();
            let _ = assignment.await.unwrap();
        }
        result = &mut assignment => {
            // An implementation may serialize the read and update, so replacement occurs later.
            result.unwrap().unwrap();
            model_db::trash_models(&db, &owner, &[model]).await.unwrap();
            sqlx::query("UPDATE models SET model_deleted = '2000-01-01T00:00:00Z' WHERE model_id = ?").bind(model).execute(&db).await.unwrap();
            assert_eq!(model_db::purge_trashed_models(&db, 30).await.unwrap(), 1);
            insert_legacy_model(&db, &foreign, blob, model).await;
        }
        () = tokio::time::sleep(Duration::from_secs(10)) => {
            resume.notify_one();
            panic!("assignment did not reach a controlled outcome");
        }
    }
    let (actual_owner, actual_group): (i64, Option<i64>) =
        sqlx::query_as("SELECT model_user_id, model_group_id FROM models WHERE model_id = ?")
            .bind(model)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(actual_owner, foreign.id);
    assert_eq!(
        actual_group, None,
        "the foreign replacement must remain ungrouped"
    );
}
