use std::collections::{BTreeMap, BTreeSet};

use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, Dot},
};

use super::{NameChoice, NameState, invalid, storage::Scope};

type Key = (String, String);

pub(super) fn key(dot: &Dot) -> Key {
    (dot.writer_id.clone(), dot.counter.clone())
}

fn dot((writer_id, counter): Key) -> Dot {
    Dot { writer_id, counter }
}

pub(super) struct Graph {
    pub frontier: Vec<Dot>,
    pub potential_tips: usize,
    choices: Vec<NameChoice>,
    missing: Vec<Dot>,
    known: BTreeSet<Key>,
}

impl Graph {
    pub const fn has_missing(&self) -> bool {
        !self.missing.is_empty()
    }

    pub fn request_parents(&self, parents: &[Dot]) -> Vec<Dot> {
        parents
            .iter()
            .filter(|parent| !self.known.contains(&key(parent)))
            .cloned()
            .collect()
    }

    pub fn state(&self, scope: &Scope) -> NameState {
        let token = super::NameToken {
            entity: scope.root.entity.clone(),
            generation: scope.generation,
            revision: scope.revision,
            frontier: self.frontier.clone(),
        };
        if scope.paused {
            return NameState::Paused {
                name: scope.current_name.clone(),
                choices: self.choices.clone(),
                token,
                missing: self.missing.clone(),
            };
        }
        if !self.missing.is_empty() {
            return NameState::Pending {
                token,
                missing: self.missing.clone(),
            };
        }
        if self.choices.len() == 1 {
            return NameState::Ready {
                name: self.choices[0].name.clone(),
                token,
            };
        }

        NameState::Conflict {
            choices: self.choices.clone(),
            token,
        }
    }

    pub fn projection(&self) -> Option<&str> {
        if self.missing.is_empty() && self.choices.len() == 1 {
            return Some(&self.choices[0].name);
        }
        // A conflict has no selected branch. Preserve the visible local value;
        // the explicit state, not that display value, describes unresolved names.
        None
    }
}

pub(super) async fn load(db: &mut sqlx::SqliteConnection, scope: &Scope) -> Result<Graph, DbError> {
    let (count, bytes): (i64, i64) = sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(length(CAST(e.envelope AS BLOB))), 0) FROM group_rename_events n JOIN causal_events e ON e.user_id = n.user_id AND e.writer_id = n.writer_id AND e.counter = n.counter WHERE n.user_id = ? AND n.birth_id = ?")
        .bind(scope.user_id).bind(&scope.birth_id).fetch_one(&mut *db).await?;
    if usize::try_from(count).map_err(|_| invalid())? > crate::causal_store::MAX_OWNER_EVENTS
        || usize::try_from(bytes)
            .map_err(|_| invalid())?
            .saturating_add(scope.root_wire.len())
            > crate::causal_store::MAX_OWNER_BYTES
    {
        return Err(invalid());
    }
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT e.encoding, e.envelope FROM group_rename_events n JOIN causal_events e ON e.user_id = n.user_id AND e.writer_id = n.writer_id AND e.counter = n.counter WHERE n.user_id = ? AND n.birth_id = ?")
        .bind(scope.user_id).bind(&scope.birth_id).fetch_all(db).await?;
    let mut nodes = BTreeMap::new();
    nodes.insert(key(&scope.root.event), scope.root.clone());
    for (encoding, wire) in rows {
        let envelope = causal_envelope::parse_causal_envelope(&wire).map_err(|_| invalid())?;
        if encoding != "portable_v2"
            || envelope.entity != scope.root.entity
            || envelope.parents.is_empty()
            || !envelope.dependencies.is_empty()
            || !matches!(
                envelope.action,
                Action::GroupRenamed { .. } | Action::GroupNameResolved { .. }
            )
            || causal_envelope::canonical_causal_envelope(&envelope).map_err(|_| invalid())? != wire
            || nodes.insert(key(&envelope.event), envelope).is_some()
        {
            return Err(invalid());
        }
    }

    reduce(&nodes, &scope.root)
}

fn extend_ancestry(
    event: &CausalEnvelope,
    indices: &BTreeMap<Key, usize>,
    ancestry: &mut [Vec<u64>],
) -> Result<(), DbError> {
    let parents = event
        .parents
        .iter()
        .map(|parent| indices.get(&key(parent)).copied().ok_or_else(invalid))
        .collect::<Result<Vec<_>, _>>()?;
    let mut inherited = vec![0_u64; ancestry.len().div_ceil(64)];
    for parent in &parents {
        for (word, prior) in inherited.iter_mut().zip(&ancestry[*parent]) {
            *word |= prior;
        }
    }
    // A superseded ancestor is not another offered choice. Real local events
    // name their frontier; allowing redundant parents could revive an old name.
    if parents
        .iter()
        .any(|parent| inherited[*parent / 64] & (1_u64 << (*parent % 64)) != 0)
    {
        return Err(invalid());
    }
    for parent in parents {
        inherited[parent / 64] |= 1_u64 << (parent % 64);
    }
    ancestry[*indices.get(&key(&event.event)).ok_or_else(invalid)?] = inherited;

    Ok(())
}

fn reduce(nodes: &BTreeMap<Key, CausalEnvelope>, root: &CausalEnvelope) -> Result<Graph, DbError> {
    let Action::GroupCreated { .. } = &root.action else {
        return Err(invalid());
    };
    let mut children: BTreeMap<Key, Vec<Key>> = BTreeMap::new();
    let mut remaining = BTreeMap::new();
    let mut missing = BTreeSet::new();
    // Pending children also reserve space in the eventual frontier. Receiving
    // their missing parent must not create a new capacity failure afterwards.
    let mut potential: BTreeSet<Key> = nodes.keys().cloned().collect();
    if nodes.len() > 1 {
        potential.remove(&key(&root.event));
    }
    let mut edges: usize = 0;
    for (id, event) in nodes {
        remaining.insert(id.clone(), event.parents.len());
        edges = edges.saturating_add(event.parents.len());
        if edges > crate::causal_store::MAX_OWNER_DEPENDENCIES {
            return Err(invalid());
        }
        for parent in &event.parents {
            let parent = key(parent);
            potential.remove(&parent);
            if !nodes.contains_key(&parent) {
                missing.insert(parent.clone());
            }
            children.entry(parent).or_default().push(id.clone());
        }
    }
    let mut ready = std::collections::VecDeque::from([key(&root.event)]);
    let mut names = BTreeMap::new();
    let mut frontier = BTreeSet::new();
    let indices = nodes.keys().cloned().zip(0..nodes.len()).collect();
    let mut ancestry = vec![Vec::new(); nodes.len()];
    while let Some(id) = ready.pop_front() {
        let event = nodes.get(&id).ok_or_else(invalid)?;
        extend_ancestry(event, &indices, &mut ancestry)?;
        let name = match &event.action {
            Action::GroupCreated { name } if id == key(&root.event) => name,
            Action::GroupRenamed { name } | Action::GroupNameResolved { name } => {
                let parent_names: BTreeSet<&String> = event
                    .parents
                    .iter()
                    .map(|parent| names.get(&key(parent)).ok_or_else(invalid))
                    .collect::<Result<_, _>>()?;
                let valid = match &event.action {
                    Action::GroupRenamed { .. } => parent_names.len() == 1,
                    Action::GroupNameResolved { .. } => {
                        parent_names.len() > 1 && parent_names.contains(name)
                    }
                    _ => false,
                };
                if !valid {
                    return Err(invalid());
                }

                name
            }
            _ => return Err(invalid()),
        };
        names.insert(id.clone(), name.clone());
        for parent in &event.parents {
            frontier.remove(&key(parent));
        }
        frontier.insert(id.clone());
        for child in children.get(&id).into_iter().flatten() {
            let count = remaining.get_mut(child).ok_or_else(invalid)?;
            *count = count.checked_sub(1).ok_or_else(invalid)?;
            if *count == 0 {
                ready.push_back(child.clone());
            }
        }
    }
    if missing.is_empty() && names.len() != nodes.len() {
        return Err(invalid());
    }
    let mut choices: BTreeMap<String, Vec<Dot>> = BTreeMap::new();
    for id in &frontier {
        choices
            .entry(names.get(id).ok_or_else(invalid)?.clone())
            .or_default()
            .push(dot(id.clone()));
    }

    Ok(Graph {
        known: nodes.keys().cloned().collect(),
        frontier: frontier.into_iter().map(dot).collect(),
        potential_tips: potential.len(),
        choices: choices
            .into_iter()
            .map(|(name, events)| NameChoice { name, events })
            .collect(),
        missing: missing.into_iter().map(dot).collect(),
    })
}
