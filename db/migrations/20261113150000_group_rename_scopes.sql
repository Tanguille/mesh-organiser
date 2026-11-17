-- This projection only covers names descended from a checked common genesis.
-- Existing bindings are deliberately not promoted: their current state may have
-- changed since observation without portable history.
CREATE TABLE group_rename_scopes (
    user_id INTEGER NOT NULL,
    entity_kind TEXT NOT NULL DEFAULT 'group' CHECK (entity_kind = 'group'),
    birth_id TEXT NOT NULL,
    guard_head INTEGER NOT NULL,
    guard_revision INTEGER NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    invalidated INTEGER NOT NULL DEFAULT 0 CHECK (invalidated IN (0, 1)),
    paused INTEGER NOT NULL DEFAULT 0 CHECK (paused IN (0, 1)),
    PRIMARY KEY (user_id, birth_id),
    FOREIGN KEY (user_id, entity_kind, birth_id) REFERENCES causal_genesis_bindings(user_id, entity_kind, birth_id) ON DELETE CASCADE
);
CREATE TABLE group_rename_events (
    user_id INTEGER NOT NULL,
    birth_id TEXT NOT NULL,
    writer_id TEXT NOT NULL,
    counter INTEGER NOT NULL,
    provenance TEXT NOT NULL CHECK (provenance IN ('local', 'checked_transfer')),
    PRIMARY KEY (writer_id, counter),
    FOREIGN KEY (user_id, birth_id) REFERENCES group_rename_scopes(user_id, birth_id) ON DELETE CASCADE,
    FOREIGN KEY (user_id, writer_id, counter) REFERENCES causal_events(user_id, writer_id, counter) ON DELETE CASCADE
);
CREATE INDEX group_rename_events_scope ON group_rename_events(user_id, birth_id);
CREATE TABLE group_rename_frontier (
    user_id INTEGER NOT NULL,
    birth_id TEXT NOT NULL,
    writer_id TEXT NOT NULL,
    counter INTEGER NOT NULL,
    PRIMARY KEY (user_id, birth_id, writer_id, counter),
    FOREIGN KEY (user_id, birth_id) REFERENCES group_rename_scopes(user_id, birth_id) ON DELETE CASCADE,
    FOREIGN KEY (user_id, writer_id, counter) REFERENCES causal_events(user_id, writer_id, counter) ON DELETE CASCADE
);
CREATE TRIGGER immutable_group_rename_event BEFORE UPDATE ON group_rename_events
BEGIN
    SELECT RAISE(ABORT, 'portable name provenance is immutable');
END;
CREATE TRIGGER irreversible_group_rename_scope BEFORE UPDATE OF invalidated ON group_rename_scopes
WHEN NEW.invalidated < OLD.invalidated
BEGIN
    SELECT RAISE(ABORT, 'name scope cannot be reactivated');
END;
CREATE TRIGGER irreversible_group_rename_pause BEFORE UPDATE OF paused ON group_rename_scopes
WHEN NEW.paused < OLD.paused
BEGIN
    SELECT RAISE(ABORT, 'name scope requires an explicit checkpoint to resume');
END;
CREATE TRIGGER initialize_group_rename_scope AFTER INSERT ON causal_genesis_bindings
WHEN NEW.entity_kind = 'group'
BEGIN
    INSERT INTO group_rename_scopes(user_id, birth_id, guard_head, guard_revision)
    VALUES (NEW.user_id, NEW.birth_id, json_extract(NEW.local_guard, '$.head'), json_extract(NEW.local_guard, '$.revision'));
    INSERT INTO group_rename_frontier(user_id, birth_id, writer_id, counter)
    VALUES (NEW.user_id, NEW.birth_id, NEW.writer_id, NEW.counter);
END;

-- Portable name writes update their expected head/revision in the same transaction.
-- All currently unsupported domain changes instead retire the scope permanently.
CREATE TRIGGER invalidate_group_name_domain AFTER UPDATE OF group_deleted_at,
    group_deletion_operation_id, group_resource_id, group_unique_global_id,
    group_user_id, group_birth_id ON models_group
WHEN OLD.group_deleted_at IS NOT NEW.group_deleted_at
    OR OLD.group_deletion_operation_id IS NOT NEW.group_deletion_operation_id
    OR OLD.group_resource_id IS NOT NEW.group_resource_id
    OR OLD.group_unique_global_id IS NOT NEW.group_unique_global_id
    OR OLD.group_user_id IS NOT NEW.group_user_id
    OR OLD.group_birth_id IS NOT NEW.group_birth_id
BEGIN
    UPDATE group_rename_scopes SET invalidated = 1
    WHERE user_id = OLD.group_user_id AND birth_id = OLD.group_birth_id;
END;
CREATE TRIGGER invalidate_deleted_group_name_scope BEFORE DELETE ON models_group
BEGIN
    UPDATE group_rename_scopes SET invalidated = 1
    WHERE user_id = OLD.group_user_id AND birth_id = OLD.group_birth_id;
END;
CREATE TRIGGER invalidate_group_name_member_insert AFTER INSERT ON models
WHEN NEW.model_group_id IS NOT NULL
BEGIN
    UPDATE group_rename_scopes SET invalidated = 1 WHERE (user_id, birth_id) IN
        (SELECT group_user_id, group_birth_id FROM models_group WHERE group_id = NEW.model_group_id);
END;
CREATE TRIGGER invalidate_group_name_member_delete BEFORE DELETE ON models
WHEN OLD.model_group_id IS NOT NULL
BEGIN
    UPDATE group_rename_scopes SET invalidated = 1 WHERE (user_id, birth_id) IN
        (SELECT group_user_id, group_birth_id FROM models_group WHERE group_id = OLD.model_group_id);
END;
CREATE TRIGGER invalidate_group_name_member_update AFTER UPDATE OF model_group_id, model_deleted, model_user_id ON models
WHEN OLD.model_group_id IS NOT NEW.model_group_id OR OLD.model_deleted IS NOT NEW.model_deleted
    OR OLD.model_user_id IS NOT NEW.model_user_id
BEGIN
    UPDATE group_rename_scopes SET invalidated = 1 WHERE (user_id, birth_id) IN
        (SELECT group_user_id, group_birth_id FROM models_group WHERE group_id IN (OLD.model_group_id, NEW.model_group_id));
END;
