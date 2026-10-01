//! `SQLite` session store for `tower-sessions` 0.14.
//!
//! Replaces `tower-sessions-sqlx-store`, which pins sqlx 0.8. Table name, columns and
//! msgpack encoding match that crate exactly, so persisted sessions stay loadable.

use async_trait::async_trait;
use sqlx::SqlitePool;
use tower_sessions_core::{
    session::{Id, Record},
    session_store::{self, Error},
};

#[derive(Clone, Debug)]
pub struct SqliteStore {
    pool: SqlitePool,
}

// Takes the error by value so it can be passed directly to `map_err`.
#[allow(clippy::needless_pass_by_value)]
fn backend_error(e: sqlx::Error) -> Error {
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

    async fn try_create(&self, record: &Record) -> session_store::Result<bool> {
        let data = rmp_serde::to_vec(record).map_err(|e| Error::Encode(e.to_string()))?;
        let result = sqlx::query(
            "insert or abort into tower_sessions (id, data, expiry_date) values (?, ?, ?)",
        )
        .bind(record.id.to_string())
        .bind(data)
        .bind(record.expiry_date)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(true),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Ok(false),
            Err(e) => Err(backend_error(e)),
        }
    }
}

#[async_trait]
impl tower_sessions_core::ExpiredDeletion for SqliteStore {
    async fn delete_expired(&self) -> session_store::Result<()> {
        sqlx::query("delete from tower_sessions where expiry_date < ?")
            .bind(time::OffsetDateTime::now_utc())
            .execute(&self.pool)
            .await
            .map_err(backend_error)?;

        Ok(())
    }
}

#[async_trait]
impl tower_sessions_core::SessionStore for SqliteStore {
    async fn create(&self, record: &mut Record) -> session_store::Result<()> {
        while !self.try_create(record).await? {
            record.id = Id::default();
        }

        Ok(())
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
        .map_err(backend_error)?;

        Ok(())
    }

    async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>> {
        let row: Option<(Vec<u8>,)> =
            sqlx::query_as("select data from tower_sessions where id = ? and expiry_date > ?")
                .bind(session_id.to_string())
                .bind(time::OffsetDateTime::now_utc())
                .fetch_optional(&self.pool)
                .await
                .map_err(backend_error)?;

        row.map(|(data,)| rmp_serde::from_slice(&data).map_err(|e| Error::Decode(e.to_string())))
            .transpose()
    }

    async fn delete(&self, session_id: &Id) -> session_store::Result<()> {
        sqlx::query("delete from tower_sessions where id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(backend_error)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // One connection so every query sees the same in-memory database.
    async fn new_store() -> super::SqliteStore {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let store = super::SqliteStore::new(pool);
        store.migrate().await.unwrap();

        store
    }

    fn record_expiring_in(offset: time::Duration) -> super::Record {
        super::Record {
            id: super::Id::default(),
            data: std::collections::HashMap::from([(
                "user".to_owned(),
                serde_json::json!({ "id": 7 }),
            )]),
            expiry_date: time::OffsetDateTime::now_utc() + offset,
        }
    }

    // Writes a row directly (msgpack blob, expiry bound as `OffsetDateTime`), bypassing `create`.
    async fn insert_raw(store: &super::SqliteStore, record: &super::Record) {
        sqlx::query("insert into tower_sessions (id, data, expiry_date) values (?, ?, ?)")
            .bind(record.id.to_string())
            .bind(rmp_serde::to_vec(record).unwrap())
            .bind(record.expiry_date)
            .execute(&store.pool)
            .await
            .unwrap();
    }

    async fn row_count(store: &super::SqliteStore) -> i64 {
        sqlx::query_scalar("select count(*) from tower_sessions")
            .fetch_one(&store.pool)
            .await
            .unwrap()
    }

    async fn load_record(store: &super::SqliteStore, id: &super::Id) -> Option<super::Record> {
        tower_sessions_core::SessionStore::load(store, id)
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
        let mut record = record_expiring_in(time::Duration::hours(1));

        tower_sessions_core::SessionStore::create(&store, &mut record)
            .await
            .unwrap();
        let loaded = load_record(&store, &record.id).await;

        assert_eq!(loaded, Some(record));
    }

    #[tokio::test]
    async fn load_unknown_id_returns_none() {
        let store = new_store().await;

        assert_eq!(load_record(&store, &super::Id::default()).await, None);
    }

    #[tokio::test]
    async fn save_inserts_then_updates() {
        let store = new_store().await;
        let mut record = record_expiring_in(time::Duration::hours(1));

        tower_sessions_core::SessionStore::save(&store, &record)
            .await
            .unwrap();
        assert_eq!(load_record(&store, &record.id).await, Some(record.clone()));

        record
            .data
            .insert("user".to_owned(), serde_json::json!("changed"));
        record.expiry_date += time::Duration::hours(1);
        tower_sessions_core::SessionStore::save(&store, &record)
            .await
            .unwrap();

        assert_eq!(row_count(&store).await, 1);
        assert_eq!(load_record(&store, &record.id).await, Some(record));
    }

    #[tokio::test]
    async fn delete_removes_record_and_is_idempotent() {
        let store = new_store().await;
        let mut record = record_expiring_in(time::Duration::hours(1));
        tower_sessions_core::SessionStore::create(&store, &mut record)
            .await
            .unwrap();

        tower_sessions_core::SessionStore::delete(&store, &record.id)
            .await
            .unwrap();
        tower_sessions_core::SessionStore::delete(&store, &record.id)
            .await
            .unwrap();

        assert_eq!(load_record(&store, &record.id).await, None);
    }

    #[tokio::test]
    async fn create_regenerates_id_on_collision() {
        let store = new_store().await;
        let mut first = record_expiring_in(time::Duration::hours(1));
        tower_sessions_core::SessionStore::create(&store, &mut first)
            .await
            .unwrap();
        let mut second = record_expiring_in(time::Duration::hours(1));
        second.id = first.id;

        tower_sessions_core::SessionStore::create(&store, &mut second)
            .await
            .unwrap();

        assert_ne!(second.id, first.id);
        assert_eq!(row_count(&store).await, 2);
        assert_eq!(load_record(&store, &first.id).await, Some(first));
        assert_eq!(load_record(&store, &second.id).await, Some(second));
    }

    #[tokio::test]
    async fn load_ignores_expired_record() {
        let store = new_store().await;
        let expired = record_expiring_in(time::Duration::hours(-1));
        insert_raw(&store, &expired).await;

        assert_eq!(load_record(&store, &expired.id).await, None);
    }

    #[tokio::test]
    async fn delete_expired_removes_only_expired_rows() {
        let store = new_store().await;
        // Seconds-scale offsets guard the boundary, not just hour-scale ones.
        let valid = record_expiring_in(time::Duration::hours(1));
        insert_raw(&store, &valid).await;
        for offset in [
            time::Duration::hours(-1),
            time::Duration::seconds(-5),
            time::Duration::seconds(30),
        ] {
            insert_raw(&store, &record_expiring_in(offset)).await;
        }

        tower_sessions_core::ExpiredDeletion::delete_expired(&store)
            .await
            .unwrap();

        assert_eq!(row_count(&store).await, 2);
        assert_eq!(load_record(&store, &valid.id).await, Some(valid));
    }
}
