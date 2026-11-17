//! Checked project snapshot identity; matching global ids alone never permits reuse.
use super::super::{encode, invalid, observation};
use crate::{DbError, model::user::User, user_db::SyncIdentity};
use sqlx::SqliteConnection;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub(super) struct Snapshot {
    pub id: i64,
    pub incarnation: String,
    pub revision: i64,
    pub global_id: String,
    pub name: String,
    pub flags: i64,
    pub created: String,
    pub modified: String,
}
#[derive(sqlx::FromRow)]
struct Binding {
    global_id: String,
    local_resource_id: i64,
    local_incarnation: String,
    local_revision: i64,
    snapshot_digest: String,
}

pub(super) async fn capture(
    db: &mut SqliteConnection,
    user: &User,
    resource: Option<i64>,
) -> Result<Option<Snapshot>, DbError> {
    let Some(id) = resource else {
        return Ok(None);
    };
    let value:Snapshot=sqlx::query_as("SELECT r.resource_id AS id,r.resource_incarnation AS incarnation,v.revision,r.resource_unique_global_id AS global_id,r.resource_name AS name,r.resource_flags AS flags,r.resource_created AS created,r.resource_last_modified AS modified FROM resources r JOIN resource_observation_versions v ON v.resource_id=r.resource_id WHERE r.resource_id=? AND r.resource_user_id=? AND (SELECT COUNT(*) FROM resources other WHERE other.resource_user_id=r.resource_user_id AND other.resource_unique_global_id=r.resource_unique_global_id)=1 AND length(CAST(r.resource_name AS BLOB))+length(CAST(r.resource_unique_global_id AS BLOB))+length(CAST(r.resource_created AS BLOB))+length(CAST(r.resource_last_modified AS BLOB))<=2097152")
        .bind(id).bind(user.id).fetch_optional(db).await?.ok_or_else(invalid)?;
    if value.incarnation.is_empty() || value.global_id.is_empty() {
        return Err(invalid());
    }
    Ok(Some(value))
}
async fn binding(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    snapshot: &Snapshot,
) -> Result<Option<Binding>, DbError> {
    sqlx::query_as("SELECT global_id,local_resource_id,local_incarnation,local_revision,snapshot_digest FROM observed_project_bindings WHERE user_id=? AND source_instance_id=? AND source_account_id=? AND source_incarnation=?")
        .bind(user.id).bind(&source.instance_id).bind(&source.account_id).bind(&snapshot.incarnation).fetch_optional(db).await
}
async fn matches(
    db: &mut SqliteConnection,
    user: &User,
    snapshot: &Snapshot,
    prior: &Binding,
) -> Result<bool, DbError> {
    if prior.global_id != snapshot.global_id
        || prior.snapshot_digest != observation::digest(&encode(snapshot)?)
    {
        return Ok(false);
    }
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM resources r JOIN resource_observation_versions v ON v.resource_id=r.resource_id WHERE r.resource_id=? AND r.resource_user_id=? AND r.resource_incarnation=? AND v.revision=? AND r.resource_unique_global_id=? AND r.resource_name=? AND r.resource_flags=? AND r.resource_created=? AND r.resource_last_modified=? AND (SELECT COUNT(*) FROM resources other WHERE other.resource_user_id=r.resource_user_id AND other.resource_unique_global_id=r.resource_unique_global_id)=1)")
        .bind(prior.local_resource_id).bind(user.id).bind(&prior.local_incarnation).bind(prior.local_revision).bind(&snapshot.global_id).bind(&snapshot.name).bind(snapshot.flags).bind(&snapshot.created).bind(&snapshot.modified).fetch_one(db).await
}
async fn record(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    snapshot: &Snapshot,
    id: i64,
) -> Result<(), DbError> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM observed_project_bindings WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&mut *db)
            .await?;
    if count >= 4096 {
        return Err(invalid());
    }
    let (incarnation,revision):(String,i64)=sqlx::query_as("SELECT r.resource_incarnation,v.revision FROM resources r JOIN resource_observation_versions v ON v.resource_id=r.resource_id WHERE r.resource_id=? AND r.resource_user_id=?")
        .bind(id).bind(user.id).fetch_one(&mut *db).await?;
    sqlx::query("INSERT INTO observed_project_bindings(user_id,source_instance_id,source_account_id,source_incarnation,global_id,local_resource_id,local_incarnation,local_revision,snapshot_digest) VALUES(?,?,?,?,?,?,?,?,?)")
        .bind(user.id).bind(&source.instance_id).bind(&source.account_id).bind(&snapshot.incarnation).bind(&snapshot.global_id).bind(id).bind(incarnation).bind(revision).bind(observation::digest(&encode(snapshot)?)).execute(db).await?;
    Ok(())
}
pub(super) async fn enroll(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    snapshot: &Snapshot,
) -> Result<(), DbError> {
    if let Some(prior) = binding(db, user, source, snapshot).await? {
        if prior.local_resource_id != snapshot.id || !matches(db, user, snapshot, &prior).await? {
            return Err(invalid());
        }
    } else {
        record(db, user, source, snapshot, snapshot.id).await?;
    }
    Ok(())
}
pub(super) async fn install(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    snapshot: &Snapshot,
) -> Result<i64, DbError> {
    if let Some(prior) = binding(db, user, source, snapshot).await? {
        if !matches(db, user, snapshot, &prior).await? {
            return Err(invalid());
        }
        return Ok(prior.local_resource_id);
    }
    let occupied:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM resources WHERE resource_user_id=? AND resource_unique_global_id=?) OR EXISTS(SELECT 1 FROM observed_project_bindings WHERE user_id=? AND global_id=?)")
        .bind(user.id).bind(&snapshot.global_id).bind(user.id).bind(&snapshot.global_id).fetch_one(&mut *db).await?;
    if occupied {
        return Err(invalid());
    }
    let highwater: i64 = sqlx::query_scalar(
        "SELECT highwater FROM object_id_highwater WHERE object_kind='resource'",
    )
    .fetch_one(&mut *db)
    .await?;
    let id = highwater.checked_add(1).ok_or_else(invalid)?;
    sqlx::query("INSERT INTO resources(resource_id,resource_name,resource_flags,resource_created,resource_user_id,resource_unique_global_id,resource_last_modified) VALUES(?,?,?,?,?,?,?)")
        .bind(id).bind(&snapshot.name).bind(snapshot.flags).bind(&snapshot.created).bind(user.id).bind(&snapshot.global_id).bind(&snapshot.modified).execute(&mut *db).await?;
    record(db, user, source, snapshot, id).await?;
    Ok(id)
}
pub(super) async fn valid(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    snapshot: &Snapshot,
) -> Result<bool, DbError> {
    let Some(prior) = binding(db, user, source, snapshot).await? else {
        return Ok(false);
    };
    matches(db, user, snapshot, &prior).await
}

/// Install the checked project before recording the group's final materialization guard.
pub(super) async fn install_association(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    snapshot: Option<&Snapshot>,
    group_id: i64,
) -> Result<(), DbError> {
    if let Some(snapshot) = snapshot {
        let resource_id = install(db, user, source, snapshot).await?;
        sqlx::query(
            "UPDATE models_group SET group_resource_id=? WHERE group_id=? AND group_user_id=?",
        )
        .bind(resource_id)
        .bind(group_id)
        .bind(user.id)
        .execute(db)
        .await?;
    }

    Ok(())
}
