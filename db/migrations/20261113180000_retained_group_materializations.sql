-- Genesis is an immutable observation. A local row may later be replaced after
-- a verified retention purge without changing that observation or portable birth.
CREATE TABLE group_lifecycle_group_rows (
    user_id INTEGER NOT NULL,
    entity_kind TEXT NOT NULL DEFAULT 'group' CHECK(entity_kind='group'),
    birth_id TEXT NOT NULL,
    local_group_id INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'present' CHECK(state IN ('present','purged')),
    deletion_operation_id TEXT,
    retained_name TEXT,
    retained_created TEXT,
    retained_last_modified TEXT,
    PRIMARY KEY(user_id,birth_id),
    UNIQUE(user_id,local_group_id),
    FOREIGN KEY(user_id,entity_kind,birth_id) REFERENCES group_lifecycle_scopes(user_id,entity_kind,birth_id) ON DELETE CASCADE,
    CHECK(state='present' OR (deletion_operation_id IS NOT NULL AND retained_name IS NOT NULL AND retained_created IS NOT NULL AND retained_last_modified IS NOT NULL))
);
-- Missing historical rows have no certificate and remain unsupported/paused.
INSERT INTO group_lifecycle_group_rows(user_id,birth_id,local_group_id)
SELECT b.user_id,b.birth_id,b.local_id FROM causal_genesis_bindings b
JOIN group_lifecycle_scopes s USING(user_id,entity_kind,birth_id)
JOIN models_group g ON g.group_id=b.local_id AND g.group_user_id=b.user_id
    AND g.group_birth_id=b.birth_id AND g.group_unique_global_id=b.global_id
WHERE b.entity_kind='group' AND b.capability='group_lifecycle_v1';

DROP TRIGGER initialize_group_lifecycle_scope;
CREATE TRIGGER initialize_group_lifecycle_scope AFTER INSERT ON causal_genesis_bindings
WHEN NEW.capability='group_lifecycle_v1'
BEGIN
    INSERT INTO group_lifecycle_scopes(user_id,entity_kind,birth_id,guard)
    VALUES(NEW.user_id,NEW.entity_kind,NEW.birth_id,NEW.local_guard);
    INSERT INTO group_lifecycle_group_rows(user_id,birth_id,local_group_id)
    SELECT NEW.user_id,NEW.birth_id,NEW.local_id WHERE NEW.entity_kind='group';
END;

DROP TRIGGER pause_combined_group_purge;
CREATE TRIGGER pause_combined_group_purge BEFORE DELETE ON models_group
WHEN NOT EXISTS (
    SELECT 1 FROM group_lifecycle_group_rows r JOIN group_lifecycle_scopes s
        ON s.user_id=r.user_id AND s.entity_kind='group' AND s.birth_id=r.birth_id
    WHERE r.user_id=OLD.group_user_id AND r.birth_id=OLD.group_birth_id
      AND r.local_group_id=OLD.group_id AND r.state='purged' AND s.paused=0
      AND r.deletion_operation_id=OLD.group_deletion_operation_id
      AND OLD.group_deleted_at IS NOT NULL AND OLD.group_resource_id IS NULL
)
BEGIN
    UPDATE group_lifecycle_scopes SET paused=1
    WHERE user_id=OLD.group_user_id AND entity_kind='group' AND birth_id=OLD.group_birth_id;
END;
