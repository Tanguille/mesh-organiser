-- An observed Trash timestamp is state, not an invented historical delete event.
ALTER TABLE model_birth_adoption_permits ADD COLUMN deleted_at TEXT;

DROP TRIGGER immutable_model_birth;
CREATE TRIGGER immutable_model_birth BEFORE UPDATE OF model_birth_id ON models
WHEN OLD.model_birth_id IS NOT NEW.model_birth_id AND NOT (
    OLD.model_birth_id IS NULL AND NEW.model_birth_id IS NOT NULL AND EXISTS(
        SELECT 1 FROM model_birth_adoption_permits p
        JOIN sync_accounts a ON a.user_id=p.user_id AND a.account_id=p.account_id
        JOIN sync_instance i ON i.singleton=1 AND i.instance_id=p.instance_id
        JOIN model_observation_versions v ON v.model_id=p.model_id AND v.revision=p.observation_revision
        WHERE p.model_id=OLD.model_id AND p.user_id=OLD.model_user_id AND p.global_id=OLD.model_unique_global_id
          AND p.incarnation=OLD.model_incarnation AND p.original_birth_id IS OLD.model_birth_id AND p.birth_id=NEW.model_birth_id
          AND p.group_id=OLD.model_group_id AND OLD.model_deleted IS p.deleted_at
          AND (SELECT COUNT(*) FROM models_labels WHERE model_id=OLD.model_id)=p.label_count
          AND NOT EXISTS(SELECT 1 FROM entity_births WHERE user_id=p.user_id AND entity_kind='model' AND (global_id=p.global_id OR birth_id=p.birth_id))
    )
)
BEGIN SELECT RAISE(ABORT,'model birth is immutable'); END;
