use crate::{
    DbError,
    db_context::DbContext,
    group_db::{GroupSyncAssignment, GroupSyncDeletion, GroupSyncGroup, GroupSyncHead},
    model::user::User,
    resource_db::ResourceSyncResource,
    user_db::SyncIdentity,
};

#[derive(serde::Serialize)]
pub struct FullSyncSnapshot {
    pub identity: SyncIdentity,
    pub groups: Vec<GroupSyncGroup>,
    pub assignments: Vec<GroupSyncAssignment>,
    pub deletions: Vec<GroupSyncDeletion>,
    pub heads: Vec<GroupSyncHead>,
    pub resources: Vec<ResourceSyncResource>,
}

pub async fn get_full_sync_snapshot(
    db: &DbContext,
    user: &User,
    expected_account_id: &str,
) -> Result<FullSyncSnapshot, DbError> {
    let mut transaction = db.begin().await?;
    let identity: Option<SyncIdentity> = sqlx::query_as("SELECT sync_instance.instance_id, sync_accounts.account_id FROM sync_instance JOIN sync_accounts ON sync_accounts.user_id = ? WHERE sync_instance.singleton = 1")
        .bind(user.id).fetch_optional(&mut *transaction).await?;
    let Some(identity) = identity.filter(|identity| {
        !identity.instance_id.is_empty()
            && !identity.account_id.is_empty()
            && identity.account_id == expected_account_id
    }) else {
        return Err(DbError::Protocol("Invalid sync account identity".into()));
    };
    let groups = crate::group_db::get_group_sync_snapshot_in(&mut transaction, user).await?;
    let resources =
        crate::resource_db::get_resource_sync_snapshot_in(&mut transaction, user).await?;
    transaction.commit().await?;
    Ok(FullSyncSnapshot {
        identity,
        groups: groups.groups,
        assignments: groups.assignments,
        deletions: groups.deletions,
        heads: groups.heads,
        resources,
    })
}
