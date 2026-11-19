use std::collections::{BTreeMap, BTreeSet};

use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, Dependency, Dot, Entity},
    causal_store,
};

use super::{MoveChoice, MoveState, invalid, storage::Scope};

type Key = (String, String);
type DestinationKey = Option<(String, String)>;

fn key(dot: &Dot) -> Key {
    (dot.writer_id.clone(), dot.counter.clone())
}
fn dot((writer_id, counter): Key) -> Dot {
    Dot { writer_id, counter }
}
fn destination_key(value: Option<&Entity>) -> Result<DestinationKey, DbError> {
    value
        .map(|group| {
            if group.kind != causal_envelope::EntityKind::Group {
                return Err(invalid());
            }
            Ok((
                group.global_id.clone(),
                group.birth_id.clone().ok_or_else(invalid)?,
            ))
        })
        .transpose()
}

pub(super) struct Graph {
    pub frontier: Vec<(Dot, Option<Entity>)>,
    pub choices: Vec<MoveChoice>,
    pub missing: Vec<Dependency>,
    known: BTreeSet<Key>,
}

impl Graph {
    pub fn state(&self, scope: &Scope) -> MoveState {
        let token = super::MoveToken {
            entity: scope.root.entity.clone(),
            incarnation: scope.incarnation.clone(),
            revision: scope.revision,
            frontier: self.frontier.iter().map(|(dot, _)| dot.clone()).collect(),
        };
        if scope.paused {
            return MoveState::Paused {
                destination: scope.current_destination.clone(),
                choices: self.choices.clone(),
                missing: self.missing.clone(),
                token,
            };
        }
        if self.choices.len() == 1 {
            return MoveState::Ready {
                destination: self.choices[0].destination.clone(),
                token,
            };
        }

        MoveState::Conflict {
            choices: self.choices.clone(),
            token,
        }
    }

    pub fn request_parents(&self, envelope: &CausalEnvelope) -> Vec<Dependency> {
        envelope
            .parents
            .iter()
            .filter(|parent| !self.known.contains(&key(parent)))
            .map(|event| Dependency {
                entity: envelope.entity.clone(),
                event: event.clone(),
            })
            .collect()
    }
}

pub(super) async fn load(db: &mut sqlx::SqliteConnection, scope: &Scope) -> Result<Graph, DbError> {
    let (count, bytes): (i64, i64) = sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(length(CAST(e.envelope AS BLOB))), 0) FROM model_move_events m JOIN causal_events e ON e.user_id = m.user_id AND e.writer_id = m.writer_id AND e.counter = m.counter WHERE m.user_id = ? AND m.birth_id = ?")
        .bind(scope.user_id).bind(&scope.birth_id).fetch_one(&mut *db).await?;
    if usize::try_from(count).map_err(|_| invalid())? > causal_store::MAX_OWNER_EVENTS
        || usize::try_from(bytes)
            .map_err(|_| invalid())?
            .saturating_add(scope.root_wire.len())
            > causal_store::MAX_OWNER_BYTES
    {
        return Err(invalid());
    }
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT e.encoding, e.envelope FROM model_move_events m JOIN causal_events e ON e.user_id = m.user_id AND e.writer_id = m.writer_id AND e.counter = m.counter WHERE m.user_id = ? AND m.birth_id = ?")
        .bind(scope.user_id).bind(&scope.birth_id).fetch_all(db).await?;
    let mut nodes = BTreeMap::from([(key(&scope.root.event), scope.root.clone())]);
    for (encoding, wire) in rows {
        let envelope = causal_envelope::parse_causal_envelope(&wire).map_err(|_| invalid())?;
        if encoding != "portable_v2"
            || envelope.entity != scope.root.entity
            || envelope.parents.is_empty()
            || !matches!(
                envelope.action,
                Action::ModelMoved { .. } | Action::ModelDestinationResolved { .. }
            )
            || causal_envelope::canonical_causal_envelope(&envelope).map_err(|_| invalid())? != wire
            || nodes.insert(key(&envelope.event), envelope).is_some()
        {
            return Err(invalid());
        }
    }

    reduce(&nodes, &scope.root)
}

fn value(
    event: &CausalEnvelope,
    values: &BTreeMap<Key, Option<Entity>>,
) -> Result<Option<Entity>, DbError> {
    let parent_values: BTreeSet<DestinationKey> = event
        .parents
        .iter()
        .map(|parent| destination_key(values.get(&key(parent)).ok_or_else(invalid)?.as_ref()))
        .collect::<Result<_, _>>()?;
    let target = match &event.action {
        Action::ModelMoved { from, to, .. }
            if parent_values.len() == 1
                && parent_values.contains(&destination_key(from.as_ref())?) =>
        {
            to
        }
        Action::ModelDestinationResolved { to, .. }
            if parent_values.len() > 1
                && parent_values.contains(&destination_key(to.as_ref())?) =>
        {
            to
        }
        _ => return Err(invalid()),
    };
    if event.dependencies.len() != usize::from(target.is_some()) {
        return Err(invalid());
    }
    destination_key(target.as_ref())?;

    Ok(target.clone())
}

fn reduce(nodes: &BTreeMap<Key, CausalEnvelope>, root: &CausalEnvelope) -> Result<Graph, DbError> {
    let mut children: BTreeMap<Key, Vec<Key>> = BTreeMap::new();
    let mut remaining = BTreeMap::new();
    let mut missing = BTreeSet::new();
    let mut edges: usize = 0;
    for (id, event) in nodes {
        remaining.insert(id.clone(), event.parents.len());
        edges = edges
            .saturating_add(event.parents.len())
            .saturating_add(event.dependencies.len());
        if edges > causal_store::MAX_OWNER_DEPENDENCIES {
            return Err(invalid());
        }
        for parent in &event.parents {
            let parent = key(parent);
            if !nodes.contains_key(&parent) {
                missing.insert(parent.clone());
            }
            children.entry(parent).or_default().push(id.clone());
        }
    }
    let mut ready = std::collections::VecDeque::from([key(&root.event)]);
    let mut values = BTreeMap::new();
    let mut frontier = BTreeSet::new();
    while let Some(id) = ready.pop_front() {
        let event = nodes.get(&id).ok_or_else(invalid)?;
        let destination = if id == key(&root.event) {
            if !matches!(event.action, Action::ModelCreated) {
                return Err(invalid());
            }

            None
        } else {
            value(event, &values)?
        };
        values.insert(id.clone(), destination);
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
    if missing.is_empty() && values.len() != nodes.len() {
        return Err(invalid());
    }
    let mut choices: BTreeMap<DestinationKey, MoveChoice> = BTreeMap::new();
    let mut tips = Vec::new();
    for id in frontier {
        let destination = values.get(&id).ok_or_else(invalid)?.clone();
        let event = dot(id);
        choices
            .entry(destination_key(destination.as_ref())?)
            .or_insert_with(|| MoveChoice {
                destination: destination.clone(),
                events: Vec::new(),
            })
            .events
            .push(event.clone());
        tips.push((event, destination));
    }

    Ok(Graph {
        frontier: tips,
        choices: choices.into_values().collect(),
        known: nodes.keys().cloned().collect(),
        missing: missing
            .into_iter()
            .map(|id| Dependency {
                entity: root.entity.clone(),
                event: dot(id),
            })
            .collect(),
    })
}
