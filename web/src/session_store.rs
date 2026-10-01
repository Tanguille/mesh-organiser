//! `SQLite` session store for `tower-sessions` 0.14.
//!
//! Replaces `tower-sessions-sqlx-store`, which pins sqlx 0.8. Table name, columns and
//! msgpack encoding match that crate exactly, so persisted sessions stay loadable.

use async_trait::async_trait;
use sqlx::SqlitePool;
use time::OffsetDateTime;
use tower_sessions::{
    ExpiredDeletion, SessionStore,
    session::{Id, Record},
    session_store::{self, Error},
};

#[derive(Clone, Debug)]
pub struct SqliteStore {
    pool: SqlitePool,
}

// Takes the error by value so it can be passed directly to `map_err`.
#[allow(clippy::needless_pass_by_value)]
fn backend(e: sqlx::Error) -> Error {
    Error::Backend(e.to_string())
}

impl SqliteStore {
    pub const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> sqlx::Result<()> {
        sqlx::query(
            "create table if not exists tower_sessions
            (id text primary key not null, data blob not null, expiry_date integer not null)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn try_create(
        conn: &mut sqlx::SqliteConnection,
        record: &Record,
    ) -> session_store::Result<bool> {
        let data = rmp_serde::to_vec(record).map_err(|e| Error::Encode(e.to_string()))?;
        let result = sqlx::query(
            "insert or abort into tower_sessions (id, data, expiry_date) values (?, ?, ?)",
        )
        .bind(record.id.to_string())
        .bind(data)
        .bind(record.expiry_date)
        .execute(conn)
        .await;

        match result {
            Ok(_) => Ok(true),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Ok(false),
            Err(e) => Err(backend(e)),
        }
    }
}

#[async_trait]
impl ExpiredDeletion for SqliteStore {
    async fn delete_expired(&self) -> session_store::Result<()> {
        sqlx::query("delete from tower_sessions where datetime(expiry_date) < datetime('now')")
            .execute(&self.pool)
            .await
            .map_err(backend)?;

        Ok(())
    }
}

#[async_trait]
impl SessionStore for SqliteStore {
    async fn create(&self, record: &mut Record) -> session_store::Result<()> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        while !Self::try_create(&mut tx, record).await? {
            record.id = Id::default();
        }
        tx.commit().await.map_err(backend)
    }

    async fn save(&self, record: &Record) -> session_store::Result<()> {
        let data = rmp_serde::to_vec(record).map_err(|e| Error::Encode(e.to_string()))?;
        sqlx::query(
            "insert into tower_sessions (id, data, expiry_date) values (?, ?, ?)
            on conflict(id) do update set
              data = excluded.data, expiry_date = excluded.expiry_date",
        )
        .bind(record.id.to_string())
        .bind(data)
        .bind(record.expiry_date)
        .execute(&self.pool)
        .await
        .map_err(backend)?;

        Ok(())
    }

    async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>> {
        let row: Option<(Vec<u8>,)> =
            sqlx::query_as("select data from tower_sessions where id = ? and expiry_date > ?")
                .bind(session_id.to_string())
                .bind(OffsetDateTime::now_utc())
                .fetch_optional(&self.pool)
                .await
                .map_err(backend)?;

        row.map(|(data,)| rmp_serde::from_slice(&data).map_err(|e| Error::Decode(e.to_string())))
            .transpose()
    }

    async fn delete(&self, session_id: &Id) -> session_store::Result<()> {
        sqlx::query("delete from tower_sessions where id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(backend)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;
    use time::{Duration, OffsetDateTime};
    use tower_sessions::{
        ExpiredDeletion, SessionStore,
        session::{Id, Record},
    };

    use super::SqliteStore;

    // One connection so every query sees the same in-memory database.
    async fn new_store() -> SqliteStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let store = SqliteStore::new(pool);
        store.migrate().await.unwrap();

        store
    }

    fn record_expiring_in(offset: Duration) -> Record {
        let mut record = Record {
            id: Id::default(),
            data: HashMap::default(),
            expiry_date: OffsetDateTime::now_utc() + offset,
        };
        record.data.insert("user".to_owned(), json!({ "id": 7 }));

        record
    }

    // Writes a row the way `tower-sessions-sqlx-store` does: msgpack blob, expiry bound as
    // `OffsetDateTime`.
    async fn insert_raw(store: &SqliteStore, record: &Record) {
        sqlx::query("insert into tower_sessions (id, data, expiry_date) values (?, ?, ?)")
            .bind(record.id.to_string())
            .bind(rmp_serde::to_vec(record).unwrap())
            .bind(record.expiry_date)
            .execute(&store.pool)
            .await
            .unwrap();
    }

    async fn row_count(store: &SqliteStore) -> i64 {
        sqlx::query_scalar("select count(*) from tower_sessions")
            .fetch_one(&store.pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn migrate_is_idempotent() {
        let store = new_store().await;

        store.migrate().await.unwrap();

        assert_eq!(row_count(&store).await, 0);
    }

    #[tokio::test]
    async fn create_then_load_round_trips() {
        let store = new_store().await;
        let mut record = record_expiring_in(Duration::hours(1));

        store.create(&mut record).await.unwrap();
        let loaded = store.load(&record.id).await.unwrap();

        assert_eq!(loaded, Some(record));
    }

    #[tokio::test]
    async fn load_unknown_id_returns_none() {
        let store = new_store().await;

        assert_eq!(store.load(&Id::default()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn save_inserts_then_updates() {
        let store = new_store().await;
        let mut record = record_expiring_in(Duration::hours(1));

        store.save(&record).await.unwrap();
        assert_eq!(store.load(&record.id).await.unwrap(), Some(record.clone()));

        record.data.insert("user".to_owned(), json!("changed"));
        record.expiry_date += Duration::hours(1);
        store.save(&record).await.unwrap();

        assert_eq!(row_count(&store).await, 1);
        assert_eq!(store.load(&record.id).await.unwrap(), Some(record));
    }

    #[tokio::test]
    async fn delete_removes_record_and_is_idempotent() {
        let store = new_store().await;
        let mut record = record_expiring_in(Duration::hours(1));
        store.create(&mut record).await.unwrap();

        store.delete(&record.id).await.unwrap();
        store.delete(&record.id).await.unwrap();

        assert_eq!(store.load(&record.id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn create_regenerates_id_on_collision() {
        let store = new_store().await;
        let mut first = record_expiring_in(Duration::hours(1));
        store.create(&mut first).await.unwrap();
        let mut second = record_expiring_in(Duration::hours(1));
        second.id = first.id;

        store.create(&mut second).await.unwrap();

        assert_ne!(second.id, first.id);
        assert_eq!(row_count(&store).await, 2);
        assert_eq!(store.load(&first.id).await.unwrap(), Some(first));
        assert_eq!(store.load(&second.id).await.unwrap(), Some(second));
    }

    #[tokio::test]
    async fn load_ignores_expired_record() {
        let store = new_store().await;
        let expired = record_expiring_in(Duration::hours(-1));
        insert_raw(&store, &expired).await;

        assert_eq!(store.load(&expired.id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn delete_expired_removes_only_expired_rows() {
        let store = new_store().await;
        let expired = record_expiring_in(Duration::hours(-1));
        let valid = record_expiring_in(Duration::hours(1));
        insert_raw(&store, &expired).await;
        insert_raw(&store, &valid).await;

        store.delete_expired().await.unwrap();

        assert_eq!(row_count(&store).await, 1);
        assert_eq!(store.load(&valid.id).await.unwrap(), Some(valid));
    }

    #[tokio::test]
    async fn loads_row_written_in_old_crate_layout() {
        let store = new_store().await;
        let record = record_expiring_in(Duration::days(1));
        insert_raw(&store, &record).await;

        assert_eq!(store.load(&record.id).await.unwrap(), Some(record));
    }
}
