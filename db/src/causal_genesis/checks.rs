use sqlx::SqliteConnection;

use crate::{
    DbError,
    causal_envelope::{self, Action, CausalEnvelope, Dot, Entity},
    causal_store::{self, IdentityStatus},
    model::user::User,
    sync_peer::PeerBinding,
    user_db::SyncIdentity,
};

use super::{GenesisBundle, counter, invalid, kind, materialization::Materialization};

pub(super) async fn genuine_root(
    db: &mut SqliteConnection,
    user: &User,
    entity: &Entity,
    materialization: &Materialization,
) -> Result<(CausalEnvelope, String), DbError> {
    let wire: String = sqlx::query_scalar("SELECT e.envelope FROM local_creation_records c JOIN causal_events e ON e.user_id = c.user_id AND e.writer_id = c.writer_id AND e.counter = c.counter JOIN entity_births b ON b.user_id = c.user_id AND b.entity_kind = c.entity_kind AND b.birth_id = c.birth_id WHERE c.user_id = ? AND c.entity_kind = ? AND c.birth_id = ? AND b.global_id = ? AND b.initialization_status = 'created' AND b.initialization_writer_id = c.writer_id AND b.initialization_counter = c.counter AND e.encoding = 'portable_v2'")
        .bind(user.id).bind(kind(entity)).bind(&entity.birth_id).bind(&entity.global_id).fetch_optional(db).await?.ok_or_else(invalid)?;
    let envelope = causal_envelope::parse_causal_envelope(&wire).map_err(|_| invalid())?;
    if envelope.entity != *entity
        || !envelope.parents.is_empty()
        || !envelope.dependencies.is_empty()
        || causal_envelope::canonical_causal_envelope(&envelope).map_err(|_| invalid())? != wire
    {
        return Err(invalid());
    }
    match (&envelope.action, materialization) {
        (Action::GroupCreated { name }, Materialization::Group(snapshot))
            if *name == snapshot.name => {}
        (Action::ModelCreated, Materialization::Model(_)) => {}
        _ => return Err(invalid()),
    }

    Ok((envelope, wire))
}

pub(super) async fn uncontested(
    db: &mut SqliteConnection,
    user: &User,
    entity: &Entity,
) -> Result<(), DbError> {
    if causal_store::identity_status(db, user.id, entity).await? != IdentityStatus::Uncontested {
        return Err(invalid());
    }

    Ok(())
}

pub(super) async fn peer(
    db: &mut SqliteConnection,
    user: &User,
    identity: &SyncIdentity,
    peer: &PeerBinding,
    bundle: &GenesisBundle,
) -> Result<(), DbError> {
    if peer.local != *identity || peer.remote != bundle.source_identity {
        return Err(invalid());
    }
    let pinned: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sync_peer_bindings WHERE user_id = ? AND local_instance_id = ? AND local_account_id = ? AND remote_instance_id = ? AND remote_account_id = ?)")
        .bind(user.id).bind(&identity.instance_id).bind(&identity.account_id).bind(&peer.remote.instance_id).bind(&peer.remote.account_id).fetch_one(db).await?;
    if !pinned {
        return Err(invalid());
    }

    Ok(())
}

pub(super) async fn no_other_history(
    db: &mut SqliteConnection,
    user: &User,
    entity: &Entity,
    root: &Dot,
) -> Result<(), DbError> {
    let present: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM causal_events WHERE user_id = ? AND encoding = 'portable_v2' AND json_extract(envelope, '$.entity.kind') = ? AND json_extract(envelope, '$.entity.global_id') = ? AND (writer_id != ? OR counter != ?)) OR EXISTS (SELECT 1 FROM causal_events WHERE user_id = ? AND encoding = 'portable_v2' AND json_extract(envelope, '$.entity.kind') = ? AND json_extract(envelope, '$.entity.birth_id') = ? AND json_extract(envelope, '$.entity.birth_id') IS NOT NULL AND (writer_id != ? OR counter != ?)) OR EXISTS (SELECT 1 FROM group_causal_events WHERE ? = 'group' AND user_id = ? AND group_global_id = ?) OR EXISTS (SELECT 1 FROM membership_causal_events WHERE ? = 'model' AND user_id = ? AND model_global_id = ?) OR EXISTS (SELECT 1 FROM received_group_renames WHERE ? = 'group' AND user_id = ? AND group_global_id = ?)")
        .bind(user.id).bind(kind(entity)).bind(&entity.global_id).bind(&root.writer_id).bind(counter(root)?)
        .bind(user.id).bind(kind(entity)).bind(&entity.birth_id).bind(&root.writer_id).bind(counter(root)?)
        .bind(kind(entity)).bind(user.id).bind(&entity.global_id)
        .bind(kind(entity)).bind(user.id).bind(&entity.global_id)
        .bind(kind(entity)).bind(user.id).bind(&entity.global_id).fetch_one(db).await?;
    if present {
        return Err(invalid());
    }

    Ok(())
}

pub(super) async fn empty_target(
    db: &mut SqliteConnection,
    user: &User,
    bundle: &GenesisBundle,
) -> Result<(), DbError> {
    let entity = bundle.entity();
    uncontested(db, user, entity).await?;
    no_other_history(db, user, entity, bundle.event()).await?;
    // An exact old birth may still refer to a purged object. Genesis installation
    // cannot resurrect it, merge a duplicate, or adopt a legacy identity.
    let occupied: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM entity_births WHERE user_id = ? AND entity_kind = ? AND (birth_id = ? OR global_id = ?)) OR EXISTS (SELECT 1 FROM models_group WHERE ? = 'group' AND group_user_id = ? AND group_unique_global_id = ?) OR EXISTS (SELECT 1 FROM models WHERE ? = 'model' AND model_user_id = ? AND model_unique_global_id = ?) OR EXISTS (SELECT 1 FROM group_sync_heads WHERE ? = 'group' AND user_id = ? AND group_global_id = ?) OR EXISTS (SELECT 1 FROM group_deletion_records WHERE ? = 'group' AND user_id = ? AND group_global_id = ?)")
        .bind(user.id).bind(kind(entity)).bind(&entity.birth_id).bind(&entity.global_id)
        .bind(kind(entity)).bind(user.id).bind(&entity.global_id)
        .bind(kind(entity)).bind(user.id).bind(&entity.global_id)
        .bind(kind(entity)).bind(user.id).bind(&entity.global_id)
        .bind(kind(entity)).bind(user.id).bind(&entity.global_id).fetch_one(db).await?;
    if occupied {
        return Err(invalid());
    }

    Ok(())
}
