use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::{GroupLifecycle, GroupState, invalid, storage::Scope};
use crate::{
    DbError,
    causal_envelope::{Action, CausalEnvelope, Dependency, Dot, Entity, MAX_CONTEXT_ITEMS},
    causal_group_rename::{NameChoice, NameState, NameToken},
    causal_model_move::{MoveChoice, MoveState, MoveToken},
};

type Key = (String, String);
fn key(dot: &Dot) -> Key {
    (dot.writer_id.clone(), dot.counter.clone())
}

pub(super) struct Graph {
    pub events: Vec<CausalEnvelope>,
    ancestors: Vec<Vec<u64>>,
    indices: BTreeMap<Key, usize>,
    order: Vec<usize>,
    observed_names: BTreeMap<Key, String>,
    observed_destinations: BTreeMap<Key, Entity>,
}

fn validate_observed_members(scopes: &[Scope], events: &[CausalEnvelope]) -> Result<(), DbError> {
    if events
        .iter()
        .any(|event| matches!(event.action, Action::LegacyObserved { .. }))
    {
        return Err(invalid());
    }
    for scope in scopes {
        for (root, batch, destination) in std::iter::once((
            &scope.root,
            &scope.observation_batch_id,
            scope.observed_destination.as_ref(),
        ))
        .chain(
            scope
                .independent_observations
                .iter()
                .map(|value| (&value.root, &value.batch_id, value.destination.as_ref())),
        ) {
            let Some(destination) = destination else {
                continue;
            };
            let target = scopes
                .iter()
                .find(|candidate| candidate.entity == *destination)
                .ok_or_else(invalid)?;
            let matches = |group: &CausalEnvelope, group_batch: &Option<String>| {
                batch.is_some()
                    && batch == group_batch
                    && group.dependencies.iter().any(|dependency| {
                        dependency.entity == scope.entity && dependency.event == root.event
                    })
            };
            if !matches(&target.root, &target.observation_batch_id)
                && !target
                    .independent_observations
                    .iter()
                    .any(|value| matches(&value.root, &value.batch_id))
            {
                return Err(invalid());
            }
        }
    }

    Ok(())
}

impl Graph {
    #[allow(clippy::too_many_lines)] // Validate the complete bounded graph before accepting any root or operation.
    pub fn new(scopes: &[Scope], mut events: Vec<CausalEnvelope>) -> Result<Self, DbError> {
        validate_observed_members(scopes, &events)?;
        events.extend(scopes.iter().map(|scope| scope.root.clone()));
        events.extend(scopes.iter().flat_map(|scope| {
            scope
                .independent_observations
                .iter()
                .map(|value| value.root.clone())
        }));
        if events.len() > 8192 {
            return Err(invalid());
        }
        let indices: BTreeMap<_, _> = events
            .iter()
            .enumerate()
            .map(|(index, event)| (key(&event.event), index))
            .collect();
        if indices.len() != events.len() {
            return Err(invalid());
        }
        let mut parents = vec![Vec::new(); events.len()];
        for (index, event) in events.iter().enumerate() {
            if !scopes.iter().any(|scope| scope.entity == event.entity) {
                return Err(invalid());
            }
            for dependency in event
                .parents
                .iter()
                .map(|dot| Dependency {
                    entity: event.entity.clone(),
                    event: dot.clone(),
                })
                .chain(event.dependencies.iter().cloned())
            {
                let parent = *indices.get(&key(&dependency.event)).ok_or_else(invalid)?;
                if events[parent].entity != dependency.entity {
                    return Err(invalid());
                }
                parents[index].push(parent);
            }
        }
        let words = events.len().div_ceil(64);
        let mut ancestors = vec![vec![0_u64; words]; events.len()];
        let mut complete = vec![false; events.len()];
        let mut order = Vec::new();
        for _ in 0..events.len() {
            let mut progress = false;
            for index in 0..events.len() {
                if complete[index] || parents[index].iter().any(|parent| !complete[*parent]) {
                    continue;
                }
                for parent in &parents[index] {
                    let inherited = ancestors[*parent].clone();
                    for (word, inherited) in ancestors[index].iter_mut().zip(inherited) {
                        *word |= inherited;
                    }
                    ancestors[index][parent / 64] |= 1_u64 << (parent % 64);
                }
                complete[index] = true;
                progress = true;
                order.push(index);
            }
            if complete.iter().all(|done| *done) {
                break;
            }
            if !progress {
                return Err(invalid());
            }
        }
        let graph = Self {
            events,
            ancestors,
            indices,
            order,
            observed_destinations: scopes
                .iter()
                .filter_map(|scope| {
                    scope
                        .observed_destination
                        .clone()
                        .map(|destination| (key(&scope.root.event), destination))
                })
                .chain(scopes.iter().flat_map(|scope| {
                    scope.independent_observations.iter().filter_map(|value| {
                        value
                            .destination
                            .clone()
                            .map(|destination| (key(&value.root.event), destination))
                    })
                }))
                .collect(),
            observed_names: scopes
                .iter()
                .filter_map(|scope| {
                    scope
                        .observed_name
                        .as_ref()
                        .map(|name| (key(&scope.root.event), name.clone()))
                })
                .chain(scopes.iter().flat_map(|scope| {
                    scope.independent_observations.iter().filter_map(|value| {
                        value
                            .name
                            .clone()
                            .map(|name| (key(&value.root.event), name))
                    })
                }))
                .collect(),
        };
        for (index, event) in graph.events.iter().enumerate() {
            graph.validate(index, event, scopes)?;
        }
        for scope in scopes {
            if graph.frontier(&scope.entity, None).len() > MAX_CONTEXT_ITEMS {
                return Err(invalid());
            }
        }

        Ok(graph)
    }

    fn sees(&self, later: usize, earlier: usize) -> bool {
        self.ancestors[later][earlier / 64] & (1_u64 << (earlier % 64)) != 0
    }
    pub fn missing_context(&self, frontier: &[Dependency]) -> Result<Vec<Dependency>, DbError> {
        let mut missing = Vec::new();
        for dependency in frontier {
            match self.indices.get(&key(&dependency.event)) {
                Some(index) if self.events[*index].entity != dependency.entity => {
                    return Err(invalid());
                }
                None if !missing.contains(dependency) => missing.push(dependency.clone()),
                Some(_) | None => {}
            }
        }

        Ok(missing)
    }
    pub fn authored_cut(
        &self,
        scopes: &[Scope],
        frontier: &[Dependency],
    ) -> Result<(Vec<Scope>, Self), DbError> {
        let mut selected = vec![false; self.events.len()];
        for dependency in frontier {
            let index = *self
                .indices
                .get(&key(&dependency.event))
                .ok_or_else(invalid)?;
            if self.events[index].entity != dependency.entity {
                return Err(invalid());
            }
            for (earlier, selected) in selected.iter_mut().enumerate() {
                *selected |= earlier == index || self.sees(index, earlier);
            }
        }
        let (scopes, cut) = self.selected_cut(scopes, &selected)?;
        let mut total = 0;
        // Group ancestry independently discovers its historic member entities.
        // Their frontiers cannot disappear just because the manifest omits them.
        for scope in &scopes {
            let mut offered: Vec<_> = frontier
                .iter()
                .filter(|dependency| dependency.entity == scope.entity)
                .map(|dependency| dependency.event.clone())
                .collect();
            offered.sort_by_key(key);
            if offered != cut.frontier(&scope.entity, None) {
                return Err(invalid());
            }
            total += offered.len();
        }
        if total != frontier.len() {
            return Err(invalid());
        }

        Ok((scopes, cut))
    }
    fn selected_cut(
        &self,
        scopes: &[Scope],
        selected: &[bool],
    ) -> Result<(Vec<Scope>, Self), DbError> {
        let scopes: Vec<_> = scopes
            .iter()
            .filter_map(|scope| {
                let included = |root: &CausalEnvelope| {
                    self.indices
                        .get(&key(&root.event))
                        .is_some_and(|index| selected[*index])
                };
                let mut scope = scope.clone();
                scope
                    .independent_observations
                    .retain(|value| included(&value.root));
                if !included(&scope.root) {
                    if scope.independent_observations.is_empty() {
                        return None;
                    }
                    let value = scope.independent_observations.remove(0);
                    scope.root = value.root;
                    scope.observed_name = value.name;
                    scope.observed_destination = value.destination;
                    scope.observation_batch_id = value.batch_id;
                }
                Some(scope)
            })
            .collect();
        let events = self
            .events
            .iter()
            .enumerate()
            .filter(|(index, event)| {
                selected[*index]
                    && !scopes.iter().any(|scope| {
                        scope.root == **event
                            || scope
                                .independent_observations
                                .iter()
                                .any(|value| value.root == **event)
                    })
            })
            .map(|(_, event)| event.clone())
            .collect();
        let cut = Self::new(&scopes, events)?;

        Ok((scopes, cut))
    }
    pub fn relevant_frontier(
        &self,
        scopes: &[Scope],
        focus: &Entity,
    ) -> Result<Vec<Dependency>, DbError> {
        let mut pending: VecDeque<_> = self.frontier(focus, None).into_iter().collect();
        let mut selected = vec![false; self.events.len()];
        let mut models = BTreeSet::new();
        while let Some(dot) = pending.pop_front() {
            let index = *self.indices.get(&key(&dot)).ok_or_else(invalid)?;
            if selected[index] {
                continue;
            }
            selected[index] = true;
            let event = &self.events[index];
            pending.extend(event.parents.iter().cloned());
            pending.extend(
                event
                    .dependencies
                    .iter()
                    .map(|dependency| dependency.event.clone()),
            );
            // Historic membership is discovered through group ancestry. Capture
            // its current model frontier, including a move away, then close over
            // those dependencies. Unrelated library roots are never added.
            if event.entity.kind == crate::causal_envelope::EntityKind::Model
                && models.insert((
                    event.entity.global_id.clone(),
                    event.entity.birth_id.clone(),
                ))
            {
                pending.extend(self.frontier(&event.entity, None));
            }
        }
        let (scopes, cut) = self.selected_cut(scopes, &selected)?;
        let mut frontier = Vec::new();
        for scope in scopes {
            frontier.extend(cut.frontier(&scope.entity, None).into_iter().map(|event| {
                Dependency {
                    entity: scope.entity.clone(),
                    event,
                }
            }));
        }

        Ok(frontier)
    }
    fn visible(&self, index: usize, past: Option<usize>) -> bool {
        past.is_none_or(|later| self.sees(later, index))
    }
    fn maximal(
        &self,
        entity: &Entity,
        past: Option<usize>,
        predicate: impl Fn(&Action) -> bool,
    ) -> Vec<usize> {
        let mut covered = vec![0_u64; self.events.len().div_ceil(64)];
        let mut maximal = Vec::new();
        for index in self.order.iter().rev().copied() {
            let event = &self.events[index];
            if event.entity != *entity
                || !self.visible(index, past)
                || !predicate(&event.action)
                || covered[index / 64] & (1_u64 << (index % 64)) != 0
            {
                continue;
            }
            maximal.push(index);
            for (word, ancestry) in covered.iter_mut().zip(&self.ancestors[index]) {
                *word |= ancestry;
            }
        }

        maximal
    }
    pub fn frontier(&self, entity: &Entity, past: Option<usize>) -> Vec<Dot> {
        let mut dots: Vec<_> = self
            .maximal(entity, past, |_| true)
            .iter()
            .map(|index| self.events[*index].event.clone())
            .collect();
        dots.sort_by_key(key);

        dots
    }
    pub fn active(&self, entity: &Entity, past: Option<usize>) -> bool {
        self.maximal(entity, past, |_| true)
            .iter()
            .any(|index| !matches!(self.events[*index].action, Action::GroupDeleted { .. }))
    }
    pub fn lifecycle(&self, entity: &Entity) -> Result<Dot, DbError> {
        let mut witnesses: Vec<_> = self
            .maximal(entity, None, |action| {
                matches!(
                    action,
                    Action::GroupCreated { .. }
                        | Action::GroupRestored { .. }
                        | Action::LegacyObserved { .. }
                )
            })
            .into_iter()
            .map(|index| self.events[index].event.clone())
            .collect();
        witnesses.sort_by_key(key);

        witnesses.into_iter().next().ok_or_else(invalid)
    }
    pub fn names(&self, entity: &Entity, past: Option<usize>) -> Vec<NameChoice> {
        let mut names: BTreeMap<String, Vec<Dot>> = BTreeMap::new();
        for index in self.maximal(entity, past, |action| {
            matches!(
                action,
                Action::GroupCreated { .. }
                    | Action::LegacyObserved { .. }
                    | Action::GroupRenamed { .. }
                    | Action::GroupNameResolved { .. }
            )
        }) {
            let name = match &self.events[index].action {
                Action::GroupCreated { name }
                | Action::GroupRenamed { name }
                | Action::GroupNameResolved { name } => name,
                Action::LegacyObserved { .. } => {
                    &self.observed_names[&key(&self.events[index].event)]
                }
                _ => unreachable!(),
            };
            names
                .entry(name.clone())
                .or_default()
                .push(self.events[index].event.clone());
        }
        names
            .into_iter()
            .map(|(name, mut events)| {
                events.sort_by_key(key);
                NameChoice { name, events }
            })
            .collect()
    }
    pub fn moves(&self, entity: &Entity, past: Option<usize>) -> Vec<MoveChoice> {
        self.moves_before_restore(entity, past, None)
    }
    fn moves_before_restore(
        &self,
        entity: &Entity,
        past: Option<usize>,
        restored: Option<&Entity>,
    ) -> Vec<MoveChoice> {
        let mut choices: Vec<MoveChoice> = Vec::new();
        for index in self.maximal(entity, past, |action| {
            matches!(
                action,
                Action::ModelCreated
                    | Action::LegacyObserved { .. }
                    | Action::ModelMoved { .. }
                    | Action::ModelDestinationResolved { .. }
                    | Action::ModelRestored { .. }
            )
        }) {
            let destination = match &self.events[index].action {
                Action::ModelMoved { to, .. } | Action::ModelDestinationResolved { to, .. } => {
                    to.clone()
                }
                Action::ModelRestored { group, .. } => Some(group.clone()),
                Action::LegacyObserved { .. } => self
                    .observed_destinations
                    .get(&key(&self.events[index].event))
                    .cloned(),
                _ => None,
            }
            .filter(|group| Some(group) != restored && self.active(group, past));
            if let Some(choice) = choices
                .iter_mut()
                .find(|choice| choice.destination == destination)
            {
                choice.events.push(self.events[index].event.clone());
            } else {
                choices.push(MoveChoice {
                    destination,
                    events: vec![self.events[index].event.clone()],
                });
            }
        }
        for choice in &mut choices {
            choice.events.sort_by_key(key);
        }
        choices.sort_by_key(|choice| {
            choice
                .destination
                .as_ref()
                .and_then(|entity| entity.birth_id.clone())
        });

        choices
    }
    fn dependency(&self, entity: &Entity, dot: &Dot) -> Result<&CausalEnvelope, DbError> {
        let event = &self.events[*self.indices.get(&key(dot)).ok_or_else(invalid)?];
        if event.entity != *entity {
            return Err(invalid());
        }

        Ok(event)
    }
    #[allow(clippy::too_many_lines)] // Keep the per-action causal checks together.
    fn validate(
        &self,
        index: usize,
        event: &CausalEnvelope,
        scopes: &[Scope],
    ) -> Result<(), DbError> {
        let prior = Some(index);
        if matches!(
            event.action,
            Action::GroupCreated { .. } | Action::ModelCreated | Action::LegacyObserved { .. }
        ) {
            if !scopes.iter().any(|scope| {
                scope.root == *event
                    || scope
                        .independent_observations
                        .iter()
                        .any(|value| value.root == *event)
            }) {
                return Err(invalid());
            }

            return Ok(());
        }
        self.validate_context(index, event, scopes)?;
        match &event.action {
            Action::GroupRenamed { .. } => {
                if !self.active(&event.entity, prior) || self.names(&event.entity, prior).len() != 1
                {
                    return Err(invalid());
                }
            }
            Action::GroupNameResolved { name } => {
                let choices = self.names(&event.entity, prior);
                if !self.active(&event.entity, prior)
                    || choices.len() < 2
                    || !choices.iter().any(|choice| choice.name == *name)
                {
                    return Err(invalid());
                }
            }
            Action::GroupDeleted { .. } => {}
            Action::GroupMemberAdded {
                model,
                membership_event,
            } => {
                let membership = self.dependency(model, membership_event)?;
                let target = match &membership.action {
                    Action::ModelMoved { to, .. } | Action::ModelDestinationResolved { to, .. } => {
                        to.as_ref()
                    }
                    Action::ModelRestored { group, .. } => Some(group),
                    _ => return Err(invalid()),
                };
                if target != Some(&event.entity) || !self.active(&event.entity, prior) {
                    return Err(invalid());
                }
            }
            Action::ModelMoved {
                from,
                to,
                target_lifecycle,
            } => {
                self.validate_lifecycle(to.as_ref(), target_lifecycle.as_ref())?;
                let choices = self.moves(&event.entity, prior);
                if choices.len() != 1
                    || choices[0].destination != *from
                    || to
                        .as_ref()
                        .is_some_and(|target| !self.active(target, prior))
                {
                    return Err(invalid());
                }
            }
            Action::ModelDestinationResolved {
                to,
                target_lifecycle,
            } => {
                self.validate_lifecycle(to.as_ref(), target_lifecycle.as_ref())?;
                let choices = self.moves(&event.entity, prior);
                if choices.len() < 2 || !choices.iter().any(|choice| choice.destination == *to) {
                    return Err(invalid());
                }
            }
            Action::ModelDetached { group, deletion } => {
                if !matches!(
                    self.dependency(group, deletion)?.action,
                    Action::GroupDeleted { .. }
                ) {
                    return Err(invalid());
                }
            }
            Action::GroupRestored { deletion } => {
                if self.active(&event.entity, prior)
                    || !matches!(
                        self.dependency(&event.entity, deletion)?.action,
                        Action::GroupDeleted { .. }
                    )
                {
                    return Err(invalid());
                }
            }
            Action::ModelRestored {
                group,
                deletion,
                restoration,
            } => {
                self.validate_restoration(event, group, deletion, restoration, prior)?;
            }
            _ => return Err(invalid()),
        }

        Ok(())
    }
    fn validate_restoration(
        &self,
        event: &CausalEnvelope,
        group: &Entity,
        deletion: &Dot,
        restoration: &Dot,
        prior: Option<usize>,
    ) -> Result<(), DbError> {
        let restore = self.dependency(group, restoration)?;
        if !matches!(&restore.action,Action::GroupRestored{deletion:original} if original==deletion)
            || !matches!(
                self.dependency(group, deletion)?.action,
                Action::GroupDeleted { .. }
            )
        {
            return Err(invalid());
        }
        let choices = self.moves_before_restore(&event.entity, prior, Some(group));
        if choices.len() != 1 || choices[0].destination.is_some() {
            return Err(invalid());
        }
        Ok(())
    }
    fn validate_lifecycle(
        &self,
        target: Option<&Entity>,
        dot: Option<&Dot>,
    ) -> Result<(), DbError> {
        match (target, dot) {
            (Some(entity), Some(dot))
                if matches!(
                    self.dependency(entity, dot)?.action,
                    Action::GroupCreated { .. }
                        | Action::GroupRestored { .. }
                        | Action::LegacyObserved { .. }
                ) =>
            {
                Ok(())
            }
            (None, None) => Ok(()),
            _ => Err(invalid()),
        }
    }
    fn validate_context(
        &self,
        index: usize,
        event: &CausalEnvelope,
        scopes: &[Scope],
    ) -> Result<(), DbError> {
        // Trash is a frozen observed facet until model lifecycle events exist.
        // Group-derived detach/recovery changes assignment without untrashing.
        if matches!(
            event.action,
            Action::ModelMoved { .. } | Action::ModelDestinationResolved { .. }
        ) && scopes
            .iter()
            .any(|scope| scope.entity == event.entity && scope.observed_deleted.is_some())
        {
            return Err(invalid());
        }
        if event.parents.is_empty()
            || !self.visible(
                *self
                    .indices
                    .get(&key(&scopes
                        .iter()
                        .find(|scope| scope.entity == event.entity)
                        .ok_or_else(invalid)?
                        .root
                        .event))
                    .ok_or_else(invalid)?,
                Some(index),
            )
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn restored_operation(&self, entity: &Entity, operation: &str) -> bool {
        self.events.iter().enumerate().any(|(index,event)| {
            let Action::GroupRestored{deletion}=&event.action else{return false;};
            if event.entity!=*entity || !self.dependency(entity,deletion).is_ok_and(|event|matches!(&event.action,Action::GroupDeleted{deletion_operation_id} if deletion_operation_id==operation)) {return false;}
            // A retry acknowledges this restore only while every known deletion
            // was observed by it. A later/concurrent deletion requires a new choice.
            self.events.iter().enumerate().filter(|(_,candidate)|candidate.entity==*entity&&matches!(candidate.action,Action::GroupDeleted{..}))
                .all(|(deletion,_)|self.sees(index,deletion))
        })
    }
    pub fn missing(&self, envelopes: &[CausalEnvelope]) -> Result<Vec<Dependency>, DbError> {
        let mut missing = Vec::new();
        for event in envelopes {
            for dependency in event
                .parents
                .iter()
                .map(|dot| Dependency {
                    entity: event.entity.clone(),
                    event: dot.clone(),
                })
                .chain(event.dependencies.iter().cloned())
            {
                let found = self
                    .events
                    .iter()
                    .chain(envelopes)
                    .find(|known| known.event == dependency.event);
                match found {
                    Some(known) if known.entity != dependency.entity => return Err(invalid()),
                    Some(_) => {}
                    None => {
                        if !missing.contains(&dependency) {
                            missing.push(dependency);
                        }
                    }
                }
            }
        }

        Ok(missing)
    }
    pub fn group_state(&self, scope: &Scope) -> GroupState {
        let token = NameToken {
            entity: scope.entity.clone(),
            generation: scope.generation,
            revision: scope.revision,
            frontier: self.frontier(&scope.entity, None),
        };
        let choices = self.names(&scope.entity, None);
        let name = if scope.paused {
            NameState::Paused {
                name: scope.current_name.clone(),
                choices,
                token,
                missing: Vec::new(),
            }
        } else if choices.len() == 1 {
            NameState::Ready {
                name: choices[0].name.clone(),
                token,
            }
        } else {
            NameState::Conflict { choices, token }
        };
        let lifecycle = if self.active(&scope.entity, None) {
            GroupLifecycle::Active
        } else {
            GroupLifecycle::Deleted {
                deletions: self.frontier(&scope.entity, None),
            }
        };

        GroupState {
            entity: scope.entity.clone(),
            lifecycle,
            name,
        }
    }
    pub fn move_state(&self, scope: &Scope) -> MoveState {
        let token = MoveToken {
            entity: scope.entity.clone(),
            incarnation: scope.incarnation.clone(),
            revision: scope.revision,
            frontier: self.frontier(&scope.entity, None),
        };
        let choices = self.moves(&scope.entity, None);
        if scope.paused {
            MoveState::Paused {
                destination: scope.current_destination.clone(),
                choices,
                token,
                missing: Vec::new(),
            }
        } else if choices.len() == 1 {
            MoveState::Ready {
                destination: choices[0].destination.clone(),
                token,
            }
        } else {
            MoveState::Conflict { choices, token }
        }
    }
}
