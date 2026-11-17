ALTER TABLE group_sync_heads ADD COLUMN identity_birth_id TEXT;
ALTER TABLE membership_causal_events ADD COLUMN model_birth_id TEXT;

-- An immutable live model incarnation proves the history's association; a global id alone does not.
UPDATE membership_causal_events SET model_birth_id = (
    SELECT m.model_birth_id FROM models m
    WHERE m.model_user_id = membership_causal_events.user_id
      AND m.model_unique_global_id = membership_causal_events.model_global_id
      AND m.model_incarnation = membership_causal_events.model_incarnation
);

-- Only a first-generation live group with its exact creation evidence can bind an old head.
UPDATE group_sync_heads SET identity_birth_id = (
    SELECT g.group_birth_id FROM models_group g
    JOIN entity_births b ON b.user_id = g.group_user_id AND b.entity_kind = 'group'
      AND b.birth_id = g.group_birth_id AND b.global_id = g.group_unique_global_id
    JOIN local_creation_records c ON c.user_id = b.user_id AND c.entity_kind = b.entity_kind
      AND c.birth_id = b.birth_id AND c.writer_id = b.initialization_writer_id
      AND c.counter = b.initialization_counter
    WHERE g.group_user_id = group_sync_heads.user_id
      AND g.group_unique_global_id = group_sync_heads.group_global_id
      AND group_sync_heads.generation = 1 AND b.initialization_status = 'created'
      AND json_extract(c.envelope, '$.entity.global_id') = g.group_unique_global_id
      AND (SELECT COUNT(*) FROM models_group other WHERE other.group_user_id = g.group_user_id
          AND other.group_unique_global_id = g.group_unique_global_id) = 1
);

CREATE TRIGGER capture_membership_birth AFTER INSERT ON membership_causal_events
BEGIN
    UPDATE membership_causal_events SET model_birth_id = (
        SELECT m.model_birth_id FROM models m
        WHERE m.model_user_id = NEW.user_id AND m.model_unique_global_id = NEW.model_global_id
          AND m.model_incarnation = NEW.model_incarnation
    ) WHERE event_id = NEW.event_id;
END;

DROP TRIGGER create_group_sync_head;
CREATE TRIGGER create_group_sync_head AFTER INSERT ON models_group
BEGIN
    INSERT INTO group_sync_heads (user_id, group_global_id, generation, head, state, identity_birth_id)
    VALUES (NEW.group_user_id, NEW.group_unique_global_id, 1, 1,
            CASE WHEN NEW.group_deleted_at IS NULL THEN 'active' ELSE 'trashed' END, NEW.group_birth_id)
    ON CONFLICT (user_id, group_global_id) DO UPDATE SET
        generation = generation + 1, head = head + 1, state = excluded.state,
        identity_birth_id = CASE WHEN group_sync_heads.identity_birth_id = excluded.identity_birth_id
            THEN group_sync_heads.identity_birth_id ELSE NULL END;
END;

DROP TRIGGER rekey_group_sync_head;
CREATE TRIGGER rekey_group_sync_head AFTER UPDATE OF group_unique_global_id ON models_group
WHEN OLD.group_unique_global_id IS NOT NEW.group_unique_global_id
BEGIN
    UPDATE group_sync_heads SET head = head + 1, state = CASE
        WHEN EXISTS (SELECT 1 FROM models_group WHERE group_user_id = OLD.group_user_id AND group_unique_global_id = OLD.group_unique_global_id AND group_deleted_at IS NULL) THEN 'active'
        WHEN EXISTS (SELECT 1 FROM models_group WHERE group_user_id = OLD.group_user_id AND group_unique_global_id = OLD.group_unique_global_id) THEN 'trashed'
        ELSE 'absent' END
    WHERE user_id = OLD.group_user_id AND group_global_id = OLD.group_unique_global_id;
    INSERT INTO group_sync_heads (user_id, group_global_id, generation, head, state, identity_birth_id)
    VALUES (NEW.group_user_id, NEW.group_unique_global_id, 1, 1,
            CASE WHEN NEW.group_deleted_at IS NULL THEN 'active' ELSE 'trashed' END, NEW.group_birth_id)
    ON CONFLICT (user_id, group_global_id) DO UPDATE SET
        generation = generation + 1, head = head + 1, state = excluded.state,
        identity_birth_id = CASE WHEN group_sync_heads.identity_birth_id = excluded.identity_birth_id
            THEN group_sync_heads.identity_birth_id ELSE NULL END;
END;

CREATE INDEX membership_birth_evidence ON membership_causal_events(user_id, model_global_id, model_birth_id);
