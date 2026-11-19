-- Capability is fixed at pristine enrollment. No prior observation is promoted.
ALTER TABLE causal_genesis_bindings ADD COLUMN capability TEXT NOT NULL
    DEFAULT 'names_moves_v1' CHECK(capability IN ('names_moves_v1', 'group_lifecycle_v1'));
DROP TRIGGER initialize_group_rename_scope;
CREATE TRIGGER initialize_group_rename_scope AFTER INSERT ON causal_genesis_bindings
WHEN NEW.entity_kind = 'group' AND NEW.capability = 'names_moves_v1'
BEGIN
    INSERT INTO group_rename_scopes(user_id, birth_id, guard_head, guard_revision)
    VALUES(NEW.user_id, NEW.birth_id, json_extract(NEW.local_guard, '$.head'), json_extract(NEW.local_guard, '$.revision'));
    INSERT INTO group_rename_frontier(user_id, birth_id, writer_id, counter)
    VALUES(NEW.user_id, NEW.birth_id, NEW.writer_id, NEW.counter);
END;
DROP TRIGGER initialize_model_move_scope;
CREATE TRIGGER initialize_model_move_scope AFTER INSERT ON causal_genesis_bindings
WHEN NEW.entity_kind = 'model' AND NEW.capability = 'names_moves_v1'
BEGIN
    INSERT INTO model_move_scopes(user_id, birth_id, guard_assignment_revision, guard_lifecycle_revision)
    VALUES(NEW.user_id, NEW.birth_id, json_extract(NEW.local_guard, '$.assignment_revision'), json_extract(NEW.local_guard, '$.lifecycle_revision'));
    INSERT INTO model_move_frontier(user_id, birth_id, writer_id, counter)
    VALUES(NEW.user_id, NEW.birth_id, NEW.writer_id, NEW.counter);
END;
DROP TRIGGER initialize_group_move_root;
CREATE TRIGGER initialize_group_move_root AFTER INSERT ON causal_genesis_bindings
WHEN NEW.entity_kind = 'group' AND NEW.capability = 'names_moves_v1'
BEGIN INSERT INTO group_move_roots(user_id, birth_id) VALUES(NEW.user_id, NEW.birth_id); END;

CREATE TABLE group_lifecycle_scopes (
    user_id INTEGER NOT NULL,
    entity_kind TEXT NOT NULL,
    birth_id TEXT NOT NULL,
    guard TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    paused INTEGER NOT NULL DEFAULT 0 CHECK(paused IN (0, 1)),
    PRIMARY KEY(user_id, entity_kind, birth_id),
    FOREIGN KEY(user_id, entity_kind, birth_id) REFERENCES causal_genesis_bindings(user_id, entity_kind, birth_id) ON DELETE CASCADE
);
CREATE TRIGGER initialize_group_lifecycle_scope AFTER INSERT ON causal_genesis_bindings
WHEN NEW.capability = 'group_lifecycle_v1'
BEGIN
    INSERT INTO group_lifecycle_scopes(user_id, entity_kind, birth_id, guard)
    VALUES(NEW.user_id, NEW.entity_kind, NEW.birth_id, NEW.local_guard);
END;
CREATE TRIGGER irreversible_group_lifecycle_pause BEFORE UPDATE OF paused ON group_lifecycle_scopes
WHEN NEW.paused < OLD.paused
BEGIN SELECT RAISE(ABORT, 'combined scope requires an explicit checkpoint'); END;

-- The manifest is metadata only. Payloads have one authority in causal_events.
CREATE TABLE group_lifecycle_operations (
    user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL,
    focus_birth_id TEXT NOT NULL,
    event_count INTEGER NOT NULL CHECK(event_count > 0),
    PRIMARY KEY(user_id, operation_id)
);
CREATE TABLE group_lifecycle_events (
    user_id INTEGER NOT NULL,
    operation_id TEXT NOT NULL,
    position INTEGER NOT NULL CHECK(position >= 0),
    writer_id TEXT NOT NULL,
    counter INTEGER NOT NULL,
    PRIMARY KEY(writer_id, counter),
    UNIQUE(user_id, operation_id, position),
    FOREIGN KEY(user_id, operation_id) REFERENCES group_lifecycle_operations(user_id, operation_id) ON DELETE CASCADE,
    FOREIGN KEY(user_id, writer_id, counter) REFERENCES causal_events(user_id, writer_id, counter) ON DELETE CASCADE
);
CREATE TRIGGER immutable_group_lifecycle_operation BEFORE UPDATE ON group_lifecycle_operations
BEGIN SELECT RAISE(ABORT, 'operation manifest is immutable'); END;
CREATE TRIGGER immutable_group_lifecycle_event BEFORE UPDATE ON group_lifecycle_events
BEGIN SELECT RAISE(ABORT, 'operation membership is immutable'); END;

-- ABA mutations outside this capability cannot be erased by restoring the old
-- values. Supported name/assignment/delete projections use revision guards.
CREATE TRIGGER pause_combined_group_identity AFTER UPDATE OF group_resource_id,group_unique_global_id,group_user_id,group_birth_id ON models_group
WHEN OLD.group_resource_id IS NOT NEW.group_resource_id OR OLD.group_unique_global_id IS NOT NEW.group_unique_global_id OR OLD.group_user_id IS NOT NEW.group_user_id OR OLD.group_birth_id IS NOT NEW.group_birth_id
BEGIN UPDATE group_lifecycle_scopes SET paused=1 WHERE user_id=OLD.group_user_id AND entity_kind='group' AND birth_id=OLD.group_birth_id; END;
CREATE TRIGGER pause_combined_model_identity AFTER UPDATE OF model_deleted,model_blob_id,model_unique_global_id,model_user_id,model_birth_id ON models
WHEN OLD.model_deleted IS NOT NEW.model_deleted OR OLD.model_blob_id IS NOT NEW.model_blob_id OR OLD.model_unique_global_id IS NOT NEW.model_unique_global_id OR OLD.model_user_id IS NOT NEW.model_user_id OR OLD.model_birth_id IS NOT NEW.model_birth_id
BEGIN UPDATE group_lifecycle_scopes SET paused=1 WHERE user_id=OLD.model_user_id AND entity_kind='model' AND birth_id=OLD.model_birth_id; END;
CREATE TRIGGER pause_combined_model_purge BEFORE DELETE ON models
BEGIN UPDATE group_lifecycle_scopes SET paused=1 WHERE user_id=OLD.model_user_id AND entity_kind='model' AND birth_id=OLD.model_birth_id; END;
CREATE TRIGGER pause_combined_group_purge BEFORE DELETE ON models_group
BEGIN UPDATE group_lifecycle_scopes SET paused=1 WHERE user_id=OLD.group_user_id AND entity_kind='group' AND birth_id=OLD.group_birth_id; END;
CREATE TRIGGER pause_combined_blob AFTER UPDATE OF blob_sha256,blob_filetype,blob_size ON blobs
WHEN OLD.blob_sha256 IS NOT NEW.blob_sha256 OR OLD.blob_filetype IS NOT NEW.blob_filetype OR OLD.blob_size IS NOT NEW.blob_size
BEGIN UPDATE group_lifecycle_scopes SET paused=1 WHERE entity_kind='model' AND (user_id,birth_id) IN (SELECT model_user_id,model_birth_id FROM models WHERE model_blob_id=NEW.blob_id); END;
CREATE TRIGGER pause_combined_unobserved_member AFTER INSERT ON models
WHEN NEW.model_group_id IS NOT NULL
BEGIN UPDATE group_lifecycle_scopes SET paused=1 WHERE entity_kind='group' AND (user_id,birth_id) IN (SELECT group_user_id,group_birth_id FROM models_group WHERE group_id=NEW.model_group_id); END;
