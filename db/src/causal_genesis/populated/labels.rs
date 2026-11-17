//! Snapshot label components without turning label globals into identity proofs.
mod storage;
use super::super::{encode, invalid, observation};
use crate::{DbError, causal_envelope::Entity, model::user::User, user_db::SyncIdentity};
use sqlx::SqliteConnection;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub(super) struct Node {
    pub id: i64,
    pub incarnation: String,
    pub revision: i64,
    pub global_id: String,
    pub name: String,
    pub color: i64,
    pub modified: String,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub(super) struct Edge {
    pub parent: String,
    pub child: String,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct Assignment {
    pub model: Entity,
    pub label: String,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub(super) struct Snapshot {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub assignments: Vec<Assignment>,
}

pub(super) async fn capture(
    db: &mut SqliteConnection,
    user: &User,
    group_id: i64,
) -> Result<Snapshot, DbError> {
    let links:Vec<(i64,String,Option<String>)>=sqlx::query_as("SELECT ml.label_id,m.model_unique_global_id,m.model_birth_id FROM models m JOIN models_labels ml ON ml.model_id=m.model_id WHERE m.model_group_id=? AND m.model_user_id=? ORDER BY m.model_unique_global_id,ml.label_id LIMIT 32769")
        .bind(group_id).bind(user.id).fetch_all(&mut *db).await?;
    if links.is_empty() {
        return Ok(Snapshot::default());
    }
    if links.len() > 32768 {
        return Err(invalid());
    }
    // Bound the owner graph before loading it; no partial hierarchy is copied.
    let (count,bytes,edges):(i64,i64,i64)=sqlx::query_as("SELECT COUNT(*),COALESCE(SUM(length(CAST(label_name AS BLOB))+length(CAST(label_unique_global_id AS BLOB))+length(CAST(label_last_modified AS BLOB))),0),(SELECT COUNT(*) FROM labels_labels e JOIN labels l ON l.label_id=e.parent_label_id OR l.label_id=e.child_label_id WHERE l.label_user_id=?) FROM labels WHERE label_user_id=?")
        .bind(user.id).bind(user.id).fetch_one(&mut *db).await?;
    if count > 4096 || bytes > 16 * 1024 * 1024 || edges > 65536 {
        return Err(invalid());
    }
    let nodes:Vec<Node>=sqlx::query_as("SELECT l.label_id AS id,l.label_incarnation AS incarnation,v.revision,l.label_unique_global_id AS global_id,l.label_name AS name,l.label_color AS color,l.label_last_modified AS modified FROM labels l JOIN label_observation_versions v ON v.label_id=l.label_id WHERE l.label_user_id=? ORDER BY l.label_incarnation")
        .bind(user.id).fetch_all(&mut *db).await?;
    let all_edges:Vec<(i64,i64)>=sqlx::query_as("SELECT parent_label_id,child_label_id FROM labels_labels WHERE parent_label_id IN (SELECT label_id FROM labels WHERE label_user_id=?) OR child_label_id IN (SELECT label_id FROM labels WHERE label_user_id=?) ORDER BY parent_label_id,child_label_id")
        .bind(user.id).bind(user.id).fetch_all(&mut *db).await?;
    let mut selected: BTreeSet<i64> = links.iter().map(|(id, _, _)| *id).collect();
    loop {
        let before = selected.len();
        for (parent, child) in &all_edges {
            if selected.contains(parent) || selected.contains(child) {
                selected.extend([parent, child]);
            }
        }
        if selected.len() == before {
            break;
        }
        if selected.len() > 4096 {
            return Err(invalid());
        }
    }
    let by_id: BTreeMap<_, _> = nodes.iter().map(|node| (node.id, node)).collect();
    if selected.iter().any(|id| !by_id.contains_key(id)) {
        return Err(invalid());
    }
    let mut globals = BTreeSet::new();
    for node in nodes.iter().filter(|node| selected.contains(&node.id)) {
        if node.incarnation.is_empty()
            || node.global_id.is_empty()
            || !globals.insert(&node.global_id)
            || nodes
                .iter()
                .filter(|other| other.global_id == node.global_id)
                .count()
                != 1
        {
            return Err(invalid());
        }
    }
    let mut result = Snapshot {
        nodes: nodes
            .iter()
            .filter(|node| selected.contains(&node.id))
            .cloned()
            .collect(),
        edges: vec![],
        assignments: vec![],
    };
    for (parent, child) in all_edges {
        if selected.contains(&parent) {
            result.edges.push(Edge {
                parent: by_id[&parent].incarnation.clone(),
                child: by_id[&child].incarnation.clone(),
            });
        }
    }
    result.edges.sort();
    for (id, global_id, birth_id) in links {
        result.assignments.push(Assignment {
            model: Entity {
                kind: crate::causal_envelope::EntityKind::Model,
                global_id,
                birth_id,
            },
            label: by_id[&id].incarnation.clone(),
        });
    }
    if encode(&result)?.len() > 2 * 1024 * 1024 {
        return Err(invalid());
    }
    Ok(result)
}

pub(super) fn bind_models(
    snapshot: &Snapshot,
    entities: &[Entity],
) -> Result<Option<Snapshot>, DbError> {
    if snapshot.nodes.is_empty() {
        return Ok(None);
    }
    let mut snapshot = snapshot.clone();
    for assignment in &mut snapshot.assignments {
        assignment.model = entities
            .iter()
            .find(|entity| entity.global_id == assignment.model.global_id)
            .cloned()
            .ok_or_else(invalid)?;
    }
    Ok(Some(snapshot))
}

fn components(snapshot: &Snapshot) -> Result<BTreeMap<String, String>, DbError> {
    let mut remaining: BTreeSet<String> = snapshot
        .nodes
        .iter()
        .map(|node| node.incarnation.clone())
        .collect();
    let mut result = BTreeMap::new();
    while let Some(first) = remaining.first().cloned() {
        let mut selected = BTreeSet::from([first]);
        loop {
            let before = selected.len();
            for edge in &snapshot.edges {
                if selected.contains(&edge.parent) || selected.contains(&edge.child) {
                    selected.extend([edge.parent.clone(), edge.child.clone()]);
                }
            }
            if before == selected.len() {
                break;
            }
        }
        let nodes: Vec<_> = snapshot
            .nodes
            .iter()
            .filter(|node| selected.contains(&node.incarnation))
            .collect();
        let edges: Vec<_> = snapshot
            .edges
            .iter()
            .filter(|edge| selected.contains(&edge.parent))
            .collect();
        let digest = observation::digest(&encode(&(nodes, edges))?);
        for incarnation in selected {
            remaining.remove(&incarnation);
            result.insert(incarnation, digest.clone());
        }
    }
    Ok(result)
}

pub(super) async fn enroll(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    snapshot: &Snapshot,
) -> Result<(), DbError> {
    storage::enroll(db, user, source, snapshot).await
}
pub(super) async fn install(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    snapshot: &Snapshot,
    models: &[(Entity, i64)],
) -> Result<(), DbError> {
    storage::install(db, user, source, snapshot, models).await
}
pub(super) async fn valid(
    db: &mut SqliteConnection,
    user: &User,
    source: &SyncIdentity,
    snapshot: &Snapshot,
) -> Result<bool, DbError> {
    storage::valid(db, user, source, snapshot).await
}
