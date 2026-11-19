use crate::util::sha256_hex;

use super::{
    DbError, Node, Snapshot, SqliteConnection, SyncIdentity, User, components, encode, invalid,
};

#[derive(sqlx::FromRow)]
struct Binding {
    global_id: String,
    local_label_id: i64,
    local_incarnation: String,
    local_revision: i64,
    snapshot_digest: String,
    component_digest: String,
}
async fn binding(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    node: &Node,
) -> Result<Option<Binding>, DbError> {
    sqlx::query_as("SELECT global_id,local_label_id,local_incarnation,local_revision,snapshot_digest,component_digest FROM observed_label_bindings WHERE user_id=? AND source_instance_id=? AND source_account_id=? AND source_incarnation=?")
        .bind(user.id).bind(&source.instance_id).bind(&source.account_id).bind(&node.incarnation).fetch_optional(db).await
}
async fn matches(
    db: &mut SqliteConnection,
    user: &User,
    node: &Node,
    component: &str,
    binding: &Binding,
) -> Result<bool, DbError> {
    if binding.global_id != node.global_id
        || binding.snapshot_digest != sha256_hex(encode(node)?.as_bytes())
        || binding.component_digest != component
    {
        return Ok(false);
    }
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM labels l JOIN label_observation_versions v ON v.label_id=l.label_id WHERE l.label_user_id=? AND l.label_id=? AND l.label_incarnation=? AND v.revision=? AND l.label_unique_global_id=? AND l.label_name=? AND l.label_color=? AND l.label_last_modified=? AND (SELECT COUNT(*) FROM labels other WHERE other.label_user_id=l.label_user_id AND other.label_unique_global_id=l.label_unique_global_id)=1)")
        .bind(user.id).bind(binding.local_label_id).bind(&binding.local_incarnation).bind(binding.local_revision).bind(&node.global_id).bind(&node.name).bind(node.color).bind(&node.modified).fetch_one(db).await
}
async fn record(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    node: &Node,
    component: &str,
    id: i64,
) -> Result<(), DbError> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM observed_label_bindings WHERE user_id=?")
            .bind(user.id)
            .fetch_one(&mut *db)
            .await?;
    if count >= 4096 {
        return Err(invalid());
    }
    let (incarnation,revision):(String,i64)=sqlx::query_as("SELECT l.label_incarnation,v.revision FROM labels l JOIN label_observation_versions v ON v.label_id=l.label_id WHERE l.label_id=? AND l.label_user_id=?")
        .bind(id).bind(user.id).fetch_one(&mut *db).await?;
    sqlx::query("INSERT INTO observed_label_bindings(user_id,source_instance_id,source_account_id,source_incarnation,global_id,local_label_id,local_incarnation,local_revision,snapshot_digest,component_digest) VALUES(?,?,?,?,?,?,?,?,?,?)")
        .bind(user.id).bind(&source.instance_id).bind(&source.account_id).bind(&node.incarnation).bind(&node.global_id).bind(id).bind(incarnation).bind(revision).bind(sha256_hex(encode(node)?.as_bytes())).bind(component).execute(db).await?;
    Ok(())
}

pub(super) async fn enroll(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    snapshot: &Snapshot,
) -> Result<(), DbError> {
    let components = components(snapshot)?;
    for node in &snapshot.nodes {
        if let Some(prior) = binding(db, user, source, node).await? {
            if prior.local_label_id != node.id
                || !matches(db, user, node, &components[&node.incarnation], &prior).await?
            {
                return Err(invalid());
            }
        } else {
            record(
                db,
                user,
                source,
                node,
                &components[&node.incarnation],
                node.id,
            )
            .await?;
        }
    }

    Ok(())
}

pub(super) async fn install(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    snapshot: &Snapshot,
    models: &[(super::Entity, i64)],
) -> Result<(), DbError> {
    let components = components(snapshot)?;
    let mut ids = super::BTreeMap::new();
    let mut fresh = Vec::new();
    for node in &snapshot.nodes {
        let id = if let Some(prior) = binding(db, user, source, node).await? {
            if !matches(db, user, node, &components[&node.incarnation], &prior).await? {
                return Err(invalid());
            }

            prior.local_label_id
        } else {
            let occupied:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM labels WHERE label_user_id=? AND label_unique_global_id=?) OR EXISTS(SELECT 1 FROM observed_label_bindings WHERE user_id=? AND global_id=?)")
                .bind(user.id).bind(&node.global_id).bind(user.id).bind(&node.global_id).fetch_one(&mut *db).await?;
            if occupied {
                return Err(invalid());
            }
            let id=sqlx::query("INSERT INTO labels(label_name,label_color,label_user_id,label_unique_global_id,label_last_modified) VALUES(?,?,?,?,?)")
                .bind(&node.name).bind(node.color).bind(user.id).bind(&node.global_id).bind(&node.modified).execute(&mut *db).await?.last_insert_rowid();
            fresh.push((node, id));
            id
        };
        ids.insert(node.incarnation.clone(), id);
    }
    for edge in &snapshot.edges {
        sqlx::query(
            "INSERT OR IGNORE INTO labels_labels(parent_label_id,child_label_id) VALUES(?,?)",
        )
        .bind(ids.get(&edge.parent).ok_or_else(invalid)?)
        .bind(ids.get(&edge.child).ok_or_else(invalid)?)
        .execute(&mut *db)
        .await?;
    }
    // Preserve link multiplicity too; duplicate source links are not normalized silently.
    for assignment in &snapshot.assignments {
        let model = models
            .iter()
            .find(|(entity, _)| *entity == assignment.model)
            .ok_or_else(invalid)?;
        sqlx::query("INSERT INTO models_labels(model_id,label_id) VALUES(?,?)")
            .bind(model.1)
            .bind(ids.get(&assignment.label).ok_or_else(invalid)?)
            .execute(&mut *db)
            .await?;
    }
    for (node, id) in fresh {
        record(db, user, source, node, &components[&node.incarnation], id).await?;
    }

    Ok(())
}

pub(super) async fn valid(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    snapshot: &Snapshot,
) -> Result<bool, DbError> {
    let components = components(snapshot)?;
    for node in &snapshot.nodes {
        let Some(prior) = binding(db, user, source, node).await? else {
            return Ok(false);
        };
        if !matches(db, user, node, &components[&node.incarnation], &prior).await? {
            return Ok(false);
        }
    }

    Ok(true)
}
