#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Group,
    Model,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    pub kind: EntityKind,
    pub global_id: String,
    #[serde(deserialize_with = "required_option")]
    pub birth_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dot {
    pub writer_id: String,
    pub counter: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub entity: Entity,
    pub event: Dot,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    GroupCreated {
        name: String,
    },
    ModelCreated,
    GroupRenamed {
        name: String,
    },
    GroupNameResolved {
        name: String,
    },
    GroupDeleted {
        deletion_operation_id: String,
    },
    GroupRestored {
        deletion: Dot,
    },
    GroupMemberAdded {
        model: Entity,
        membership_event: Dot,
    },
    ModelMoved {
        #[serde(deserialize_with = "required_option")]
        from: Option<Entity>,
        #[serde(deserialize_with = "required_option")]
        to: Option<Entity>,
        #[serde(deserialize_with = "required_option")]
        target_lifecycle: Option<Dot>,
    },
    ModelDestinationResolved {
        #[serde(deserialize_with = "required_option")]
        to: Option<Entity>,
        #[serde(deserialize_with = "required_option")]
        target_lifecycle: Option<Dot>,
    },
    ModelDetached {
        group: Entity,
        deletion: Dot,
    },
    ModelRestored {
        group: Entity,
        deletion: Dot,
        restoration: Dot,
    },
    LegacyObserved {
        observation_digest: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CausalEnvelope {
    pub version: u32,
    pub entity: Entity,
    pub event: Dot,
    pub parents: Vec<Dot>,
    pub dependencies: Vec<Dependency>,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreationCompleteness {
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalCreationRecord {
    pub envelope: CausalEnvelope,
    pub completeness: CreationCompleteness,
}

pub async fn get_local_creation_record(
    db: &crate::db_context::DbContext,
    user: &crate::model::user::User,
    kind: EntityKind,
    birth_id: &str,
) -> Result<Option<LocalCreationRecord>, crate::DbError> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT e.envelope FROM local_creation_records c JOIN causal_events e ON e.user_id = c.user_id AND e.writer_id = c.writer_id AND e.counter = c.counter WHERE c.user_id = ? AND c.entity_kind = ? AND c.birth_id = ? AND c.completeness = 'incomplete'",
    )
    .bind(user.id)
    .bind(match kind { EntityKind::Group => "group", EntityKind::Model => "model" })
    .bind(birth_id)
    .fetch_optional(db)
    .await?;
    value
        .map(|value| {
            let envelope = parse_causal_envelope(&value)
                .map_err(|_| crate::DbError::Protocol("Invalid local creation record".into()))?;
            Ok(LocalCreationRecord {
                envelope,
                completeness: CreationCompleteness::Incomplete,
            })
        })
        .transpose()
}

pub(crate) async fn record_local_creation(
    db: &mut sqlx::SqliteConnection,
    user: &crate::model::user::User,
    entity: Entity,
    action: Action,
) -> Result<(), crate::DbError> {
    let (writer_id, counter) = crate::causal_clock::next(db, user.id).await?;
    let envelope = CausalEnvelope {
        version: 2,
        entity,
        event: Dot {
            writer_id,
            counter: counter.to_string(),
        },
        parents: Vec::new(),
        dependencies: Vec::new(),
        action,
    };
    let value = canonical_causal_envelope(&envelope)
        .map_err(|_| crate::DbError::Protocol("Invalid local creation record".into()))?;
    let kind = match envelope.entity.kind {
        EntityKind::Group => "group",
        EntityKind::Model => "model",
    };
    sqlx::query(
        "INSERT INTO causal_events (user_id, writer_id, counter, envelope) VALUES (?, ?, ?, ?)",
    )
    .bind(user.id)
    .bind(&envelope.event.writer_id)
    .bind(counter)
    .bind(value)
    .execute(&mut *db)
    .await?;
    sqlx::query("INSERT INTO local_creation_records (user_id, entity_kind, birth_id, writer_id, counter) VALUES (?, ?, ?, ?, ?)")
        .bind(user.id).bind(kind).bind(&envelope.entity.birth_id).bind(&envelope.event.writer_id).bind(counter).execute(&mut *db).await?;
    sqlx::query("UPDATE entity_births SET initialization_status = 'created', initialization_writer_id = ?, initialization_counter = ? WHERE user_id = ? AND entity_kind = ? AND birth_id = ?")
        .bind(&envelope.event.writer_id).bind(counter).bind(user.id).bind(kind).bind(&envelope.entity.birth_id).execute(db).await?;
    Ok(())
}

#[derive(Debug)]
pub struct EnvelopeError;

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Invalid causal envelope")
    }
}

impl std::error::Error for EnvelopeError {}

pub fn parse_causal_envelope(value: &str) -> Result<CausalEnvelope, EnvelopeError> {
    if value.len() > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeError);
    }
    let mut envelope: CausalEnvelope = serde_json::from_str(value).map_err(|_| EnvelopeError)?;
    validate(&envelope)?;
    sort_context(&mut envelope);
    Ok(envelope)
}

pub fn canonical_causal_envelope(value: &CausalEnvelope) -> Result<String, EnvelopeError> {
    validate(value)?;
    let mut canonical = value.clone();
    sort_context(&mut canonical);
    let encoded = serde_json::to_string(&canonical).map_err(|_| EnvelopeError)?;
    if encoded.len() > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeError);
    }
    Ok(encoded)
}

pub const MAX_ENVELOPE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CONTEXT_ITEMS: usize = 1024;
pub const MAX_NAME_BYTES: usize = 65_536;

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(deserializer)
}

fn validate_entity(entity: &Entity, allow_unknown_birth: bool) -> Result<(), EnvelopeError> {
    if entity.global_id.len() != 32
        || entity
            .birth_id
            .as_ref()
            .map_or(!allow_unknown_birth, |id| !crate::util::is_blob_sha256(id))
    {
        return Err(EnvelopeError);
    }
    Ok(())
}

pub(crate) fn counter(dot: &Dot) -> Result<i64, EnvelopeError> {
    if !crate::util::is_blob_sha256(&dot.writer_id)
        || dot.counter.starts_with('0')
        || !dot.counter.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(EnvelopeError);
    }
    dot.counter
        .parse::<i64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or(EnvelopeError)
}

fn sort_context(value: &mut CausalEnvelope) {
    value
        .parents
        .sort_by(|a, b| (&a.writer_id, &a.counter).cmp(&(&b.writer_id, &b.counter)));
    value.dependencies.sort_by(|a, b| {
        (&a.event.writer_id, &a.event.counter).cmp(&(&b.event.writer_id, &b.event.counter))
    });
}

fn validate(value: &CausalEnvelope) -> Result<(), EnvelopeError> {
    use std::collections::HashSet;
    if value.version != 2
        || value.parents.len() > MAX_CONTEXT_ITEMS
        || value.dependencies.len() > MAX_CONTEXT_ITEMS
    {
        return Err(EnvelopeError);
    }
    let legacy = matches!(value.action, Action::LegacyObserved { .. });
    validate_entity(&value.entity, legacy)?;
    let own_counter = counter(&value.event)?;
    let mut dots = HashSet::new();
    for dot in &value.parents {
        let parent_counter = counter(dot)?;
        if (dot.writer_id == value.event.writer_id && parent_counter >= own_counter)
            || !dots.insert((&dot.writer_id, parent_counter))
        {
            return Err(EnvelopeError);
        }
    }
    let mut dependency_dots = HashSet::new();
    for dependency in &value.dependencies {
        validate_entity(&dependency.entity, false)?;
        let dependency_counter = counter(&dependency.event)?;
        let dot = (&dependency.event.writer_id, dependency_counter);
        let restore_parent = matches!(
            &value.action,
            Action::GroupRestored { deletion }
                if deletion == &dependency.event && dependency.entity == value.entity
        );
        if (dependency.event.writer_id == value.event.writer_id
            && dependency_counter >= own_counter)
            || !dependency_dots.insert(dot)
            || (dots.contains(&dot) && !restore_parent)
        {
            return Err(EnvelopeError);
        }
    }
    validate_action(value)
}

fn validate_action(value: &CausalEnvelope) -> Result<(), EnvelopeError> {
    let require = |entity: &Entity, dot: &Dot| -> Result<(), EnvelopeError> {
        if value
            .dependencies
            .iter()
            .any(|dependency| &dependency.entity == entity && &dependency.event == dot)
        {
            Ok(())
        } else {
            Err(EnvelopeError)
        }
    };
    match &value.action {
        Action::ModelDestinationResolved {
            to,
            target_lifecycle,
        } => validate_destination_resolution(value, to.as_ref(), target_lifecycle.as_ref())?,
        Action::GroupNameResolved { name } => validate_name_resolution(value, name)?,
        Action::GroupCreated { name } | Action::GroupRenamed { name } => {
            if value.entity.kind != EntityKind::Group || name.len() > MAX_NAME_BYTES {
                return Err(EnvelopeError);
            }
        }
        Action::ModelCreated => {
            if value.entity.kind != EntityKind::Model {
                return Err(EnvelopeError);
            }
        }
        Action::GroupDeleted {
            deletion_operation_id,
        } => {
            if value.entity.kind != EntityKind::Group
                || !crate::util::is_blob_sha256(deletion_operation_id)
            {
                return Err(EnvelopeError);
            }
        }
        Action::GroupRestored { deletion } => {
            if value.entity.kind != EntityKind::Group {
                return Err(EnvelopeError);
            }
            require(&value.entity, deletion)?;
        }
        Action::GroupMemberAdded {
            model,
            membership_event,
        } => {
            if value.entity.kind != EntityKind::Group || model.kind != EntityKind::Model {
                return Err(EnvelopeError);
            }
            validate_entity(model, false)?;
            require(model, membership_event)?;
        }
        Action::ModelMoved {
            from,
            to,
            target_lifecycle,
        } => {
            if value.entity.kind != EntityKind::Model || from == to {
                return Err(EnvelopeError);
            }
            for group in from.iter().chain(to.iter()) {
                if group.kind != EntityKind::Group {
                    return Err(EnvelopeError);
                }
                validate_entity(group, false)?;
            }
            match (to, target_lifecycle) {
                (Some(group), Some(dot)) => require(group, dot)?,
                (None, None) => {}
                _ => return Err(EnvelopeError),
            }
        }
        Action::ModelDetached { group, deletion } => {
            if value.entity.kind != EntityKind::Model || group.kind != EntityKind::Group {
                return Err(EnvelopeError);
            }
            validate_entity(group, false)?;
            require(group, deletion)?;
        }
        Action::ModelRestored {
            group,
            deletion,
            restoration,
        } => {
            if value.entity.kind != EntityKind::Model
                || group.kind != EntityKind::Group
                || deletion == restoration
            {
                return Err(EnvelopeError);
            }
            validate_entity(group, false)?;
            require(group, deletion)?;
            require(group, restoration)?;
        }
        Action::LegacyObserved { observation_digest } => {
            if observation_digest.is_empty() || observation_digest.len() > MAX_NAME_BYTES {
                return Err(EnvelopeError);
            }
        }
    }
    Ok(())
}

fn validate_name_resolution(value: &CausalEnvelope, name: &str) -> Result<(), EnvelopeError> {
    if value.entity.kind != EntityKind::Group
        || name.len() > MAX_NAME_BYTES
        || value.parents.len() < 2
        || !value.dependencies.is_empty()
    {
        return Err(EnvelopeError);
    }

    Ok(())
}

fn validate_destination_resolution(
    value: &CausalEnvelope,
    to: Option<&Entity>,
    lifecycle: Option<&Dot>,
) -> Result<(), EnvelopeError> {
    if value.entity.kind != EntityKind::Model || value.parents.len() < 2 {
        return Err(EnvelopeError);
    }
    match (to, lifecycle) {
        // Combined lifecycle choices retain observed deletion/keep context even
        // when the selected destination is ungrouped. Scoped reducers validate
        // those references; the narrow move reducer still requires no extras.
        (None, None) => {}
        (Some(group), Some(event)) if group.kind == EntityKind::Group => {
            validate_entity(group, false)?;
            if !value
                .dependencies
                .iter()
                .any(|dependency| dependency.entity == *group && dependency.event == *event)
            {
                return Err(EnvelopeError);
            }
        }
        _ => return Err(EnvelopeError),
    }

    Ok(())
}
