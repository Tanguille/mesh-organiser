ALTER TABLE group_deletion_records ADD COLUMN group_birth_id TEXT;

CREATE TABLE group_birth_rehydration_permits (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    birth_id TEXT NOT NULL,
    global_id TEXT NOT NULL,
    deletion_operation_id TEXT NOT NULL,
    PRIMARY KEY (user_id, birth_id)
);

DROP TRIGGER register_group_birth;
CREATE TRIGGER register_group_birth AFTER INSERT ON models_group WHEN NEW.group_birth_id IS NOT NULL
BEGIN
    SELECT CASE WHEN EXISTS (
        SELECT 1 FROM entity_births b WHERE b.user_id = NEW.group_user_id
          AND b.entity_kind = 'group' AND b.birth_id = NEW.group_birth_id
    ) AND NOT EXISTS (
        SELECT 1 FROM group_birth_rehydration_permits p
        JOIN entity_births b ON b.user_id = p.user_id AND b.entity_kind = 'group'
          AND b.birth_id = p.birth_id AND b.global_id = p.global_id
        JOIN group_deletion_records d ON d.user_id = p.user_id AND d.group_global_id = p.global_id
          AND d.operation_id = p.deletion_operation_id AND d.group_birth_id = p.birth_id AND d.restored_at IS NULL
        JOIN group_sync_heads h ON h.user_id = p.user_id AND h.group_global_id = p.global_id
          AND h.last_deletion_operation_id = p.deletion_operation_id
        WHERE p.user_id = NEW.group_user_id AND p.birth_id = NEW.group_birth_id
          AND p.global_id = NEW.group_unique_global_id
    ) THEN RAISE(ABORT, 'group birth already registered') END;
    INSERT INTO entity_births(user_id, entity_kind, birth_id, global_id)
    SELECT NEW.group_user_id, 'group', NEW.group_birth_id, NEW.group_unique_global_id
    WHERE NOT EXISTS (SELECT 1 FROM entity_births WHERE user_id = NEW.group_user_id
        AND entity_kind = 'group' AND birth_id = NEW.group_birth_id);
    DELETE FROM group_birth_rehydration_permits WHERE user_id = NEW.group_user_id
      AND birth_id = NEW.group_birth_id AND global_id = NEW.group_unique_global_id;
END;
