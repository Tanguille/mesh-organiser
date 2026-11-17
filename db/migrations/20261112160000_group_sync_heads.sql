CREATE TABLE group_sync_heads (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    group_global_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    head INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('active', 'trashed', 'absent')),
    PRIMARY KEY (user_id, group_global_id)
);

-- Legacy duplicate identities remain detectable by the snapshot rather than blocking upgrade.
INSERT OR IGNORE INTO group_sync_heads (user_id, group_global_id, generation, head, state)
SELECT group_user_id, group_unique_global_id, 1, MAX(group_revision + 1, 1),
       CASE WHEN group_deleted_at IS NULL THEN 'active' ELSE 'trashed' END
FROM models_group ORDER BY group_id;

INSERT OR IGNORE INTO group_sync_heads (user_id, group_global_id, generation, head, state)
SELECT user_id, group_global_id, 1, MAX(MAX(group_revision) + 1, 1), 'absent'
FROM group_deletion_records GROUP BY user_id, group_global_id;

CREATE TRIGGER create_group_sync_head AFTER INSERT ON models_group
BEGIN
    INSERT INTO group_sync_heads (user_id, group_global_id, generation, head, state)
    VALUES (NEW.group_user_id, NEW.group_unique_global_id, 1, 1,
            CASE WHEN NEW.group_deleted_at IS NULL THEN 'active' ELSE 'trashed' END)
    ON CONFLICT (user_id, group_global_id) DO UPDATE SET
        generation = generation + 1, head = head + 1, state = excluded.state;
END;

CREATE TRIGGER revise_group_sync_head AFTER UPDATE OF group_revision ON models_group
WHEN OLD.group_revision IS NOT NEW.group_revision
BEGIN
    UPDATE group_sync_heads SET head = head + 1,
        state = CASE WHEN NEW.group_deleted_at IS NULL THEN 'active' ELSE 'trashed' END
    WHERE user_id = NEW.group_user_id AND group_global_id = NEW.group_unique_global_id;
END;

CREATE TRIGGER rekey_group_sync_head AFTER UPDATE OF group_unique_global_id ON models_group
WHEN OLD.group_unique_global_id IS NOT NEW.group_unique_global_id
BEGIN
    UPDATE group_sync_heads SET head = head + 1, state = CASE
        WHEN EXISTS (SELECT 1 FROM models_group WHERE group_user_id = OLD.group_user_id AND group_unique_global_id = OLD.group_unique_global_id AND group_deleted_at IS NULL) THEN 'active'
        WHEN EXISTS (SELECT 1 FROM models_group WHERE group_user_id = OLD.group_user_id AND group_unique_global_id = OLD.group_unique_global_id) THEN 'trashed'
        ELSE 'absent' END
    WHERE user_id = OLD.group_user_id AND group_global_id = OLD.group_unique_global_id;
    INSERT INTO group_sync_heads (user_id, group_global_id, generation, head, state)
    VALUES (NEW.group_user_id, NEW.group_unique_global_id, 1, 1,
            CASE WHEN NEW.group_deleted_at IS NULL THEN 'active' ELSE 'trashed' END)
    ON CONFLICT (user_id, group_global_id) DO UPDATE SET
        generation = generation + 1, head = head + 1, state = excluded.state;
END;

CREATE TRIGGER retire_group_sync_head AFTER DELETE ON models_group
BEGIN
    UPDATE group_sync_heads SET head = head + 1, state = CASE
        WHEN EXISTS (SELECT 1 FROM models_group WHERE group_user_id = OLD.group_user_id AND group_unique_global_id = OLD.group_unique_global_id AND group_deleted_at IS NULL) THEN 'active'
        WHEN EXISTS (SELECT 1 FROM models_group WHERE group_user_id = OLD.group_user_id AND group_unique_global_id = OLD.group_unique_global_id) THEN 'trashed'
        ELSE 'absent' END
    WHERE user_id = OLD.group_user_id AND group_global_id = OLD.group_unique_global_id;
END;
