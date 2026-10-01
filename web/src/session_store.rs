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
