//! Decoder used only after the hub authenticates the new event's writer credential.

use crate::{
    DbError,
    causal_envelope::{self, EntityKind},
    user_db::SyncIdentity,
};

use super::{
    GenesisBundle, invalid,
    materialization::{self, Materialization},
    storage,
};

pub fn encode_group(bundle: &GenesisBundle) -> Result<serde_json::Value, DbError> {
    if bundle.capability != "group_lifecycle_v1"
        || bundle.observation_evidence.is_some()
        || bundle.entity().kind != EntityKind::Group
    {
        return Err(invalid());
    }
    let Materialization::Group(snapshot) = &bundle.materialization else {
        return Err(invalid());
    };

    Ok(serde_json::json!({"kind":"group_genesis","root":bundle.wire,"materialization":snapshot}))
}

pub async fn install_group(
    db: &mut sqlx::SqliteConnection,
    user: &crate::model::user::User,
    identity: &SyncIdentity,
    source: &SyncIdentity,
    wire: &str,
    snapshot: serde_json::Value,
) -> Result<(), DbError> {
    let envelope = causal_envelope::parse_causal_envelope(wire).map_err(|_| invalid())?;
    let snapshot: materialization::GroupSnapshot =
        serde_json::from_value(snapshot).map_err(|_| invalid())?;
    if envelope.entity.kind != EntityKind::Group
        || envelope.entity.birth_id.is_none()
        || !envelope.parents.is_empty()
        || !envelope.dependencies.is_empty()
        || causal_envelope::canonical_causal_envelope(&envelope).map_err(|_| invalid())? != wire
        || !matches!(&envelope.action,causal_envelope::Action::GroupCreated{name} if name==&snapshot.name)
    {
        return Err(invalid());
    }
    let materialization = Materialization::Group(snapshot);
    let bundle = GenesisBundle {
        envelope,
        wire: wire.into(),
        source_identity: source.clone(),
        materialization_json: super::encode(&materialization)?,
        materialization,
        capability: "group_lifecycle_v1".into(),
        observation_evidence: None,
    };
    // This is a new authenticated root. Occupied targets require a separate adoption protocol.
    super::checks::empty_target(db, user, &bundle).await?;
    storage::store_received_root(db, user, &bundle).await?;
    let (id, guard) =
        materialization::install(db, user, bundle.entity(), &bundle.materialization, None).await?;
    storage::write_binding(db, user, identity, &bundle, id, &guard, "installed").await?;

    Ok(())
}
