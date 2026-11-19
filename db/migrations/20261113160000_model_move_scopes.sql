-- Membership is a separate verified scope. Group names/lifecycle remain incomplete;
-- membership changes still invalidate the earlier rename-only projection.
CREATE INDEX causal_genesis_local_entity ON causal_genesis_bindings(user_id, entity_kind, local_id);
CREATE TABLE group_move_roots (
    user_id INTEGER NOT NULL,
    entity_kind TEXT NOT NULL DEFAULT 'group' CHECK(entity_kind = 'group'),
    birth_id TEXT NOT NULL,
    invalidated INTEGER NOT NULL DEFAULT 0 CHECK(invalidated IN (0, 1)),
    PRIMARY KEY(user_id, birth_id),
    FOREIGN KEY(user_id, entity_kind, birth_id) REFERENCES causal_genesis_bindings(user_id, entity_kind, birth_id) ON DELETE CASCADE
);
CREATE TABLE model_move_scopes (
    user_id INTEGER NOT NULL,
    entity_kind TEXT NOT NULL DEFAULT 'model' CHECK(entity_kind = 'model'),
    birth_id TEXT NOT NULL,
    guard_assignment_revision INTEGER NOT NULL,
    guard_lifecycle_revision INTEGER NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    invalidated INTEGER NOT NULL DEFAULT 0 CHECK(invalidated IN (0, 1)),
    paused INTEGER NOT NULL DEFAULT 0 CHECK(paused IN (0, 1)),
    PRIMARY KEY(user_id, birth_id),
    FOREIGN KEY(user_id, entity_kind, birth_id) REFERENCES causal_genesis_bindings(user_id, entity_kind, birth_id) ON DELETE CASCADE
);
CREATE TABLE model_move_events (
    user_id INTEGER NOT NULL,
    birth_id TEXT NOT NULL,
    writer_id TEXT NOT NULL,
    counter INTEGER NOT NULL,
    provenance TEXT NOT NULL CHECK(provenance IN ('local', 'checked_transfer')),
    PRIMARY KEY(writer_id, counter),
    FOREIGN KEY(user_id, birth_id) REFERENCES model_move_scopes(user_id, birth_id) ON DELETE CASCADE,
    FOREIGN KEY(user_id, writer_id, counter) REFERENCES causal_events(user_id, writer_id, counter) ON DELETE CASCADE
);
CREATE INDEX model_move_events_scope ON model_move_events(user_id, birth_id);
CREATE TABLE model_move_frontier (
    user_id INTEGER NOT NULL,
    birth_id TEXT NOT NULL,
    writer_id TEXT NOT NULL,
    counter INTEGER NOT NULL,
    target_birth_id TEXT,
    PRIMARY KEY(user_id, birth_id, writer_id, counter),
    FOREIGN KEY(user_id, birth_id) REFERENCES model_move_scopes(user_id, birth_id) ON DELETE CASCADE,
    FOREIGN KEY(user_id, writer_id, counter) REFERENCES causal_events(user_id, writer_id, counter) ON DELETE CASCADE,
    FOREIGN KEY(user_id, target_birth_id) REFERENCES group_move_roots(user_id, birth_id)
);
CREATE INDEX model_move_frontier_target ON model_move_frontier(user_id, target_birth_id);
CREATE TRIGGER immutable_model_move_event BEFORE UPDATE ON model_move_events
BEGIN SELECT RAISE(ABORT, 'portable move provenance is immutable'); END;
CREATE TRIGGER irreversible_model_move_scope BEFORE UPDATE OF invalidated, paused ON model_move_scopes
WHEN NEW.invalidated < OLD.invalidated OR NEW.paused < OLD.paused
BEGIN SELECT RAISE(ABORT, 'model move scope requires an explicit checkpoint'); END;
CREATE TRIGGER irreversible_group_move_root BEFORE UPDATE OF invalidated ON group_move_roots
WHEN NEW.invalidated < OLD.invalidated
BEGIN SELECT RAISE(ABORT, 'group move root cannot be reactivated'); END;

CREATE TRIGGER initialize_model_move_scope AFTER INSERT ON causal_genesis_bindings
WHEN NEW.entity_kind = 'model'
BEGIN
    INSERT INTO model_move_scopes(user_id, birth_id, guard_assignment_revision, guard_lifecycle_revision)
    VALUES(NEW.user_id, NEW.birth_id, json_extract(NEW.local_guard, '$.assignment_revision'), json_extract(NEW.local_guard, '$.lifecycle_revision'));
    INSERT INTO model_move_frontier(user_id, birth_id, writer_id, counter)
    VALUES(NEW.user_id, NEW.birth_id, NEW.writer_id, NEW.counter);
END;
CREATE TRIGGER initialize_group_move_root AFTER INSERT ON causal_genesis_bindings
WHEN NEW.entity_kind = 'group'
BEGIN INSERT INTO group_move_roots(user_id, birth_id) VALUES(NEW.user_id, NEW.birth_id); END;

CREATE TRIGGER invalidate_group_move_root AFTER UPDATE OF group_deleted_at, group_deletion_operation_id,
    group_resource_id, group_unique_global_id, group_user_id, group_birth_id ON models_group
WHEN OLD.group_deleted_at IS NOT NEW.group_deleted_at
    OR OLD.group_deletion_operation_id IS NOT NEW.group_deletion_operation_id
    OR OLD.group_resource_id IS NOT NEW.group_resource_id
    OR OLD.group_unique_global_id IS NOT NEW.group_unique_global_id
    OR OLD.group_user_id IS NOT NEW.group_user_id
    OR OLD.group_birth_id IS NOT NEW.group_birth_id
BEGIN
    UPDATE group_move_roots SET invalidated = 1 WHERE user_id = OLD.group_user_id AND birth_id = OLD.group_birth_id;
END;
CREATE TRIGGER invalidate_deleted_group_move_root BEFORE DELETE ON models_group
BEGIN
    UPDATE group_move_roots SET invalidated = 1 WHERE user_id = OLD.group_user_id AND birth_id = OLD.group_birth_id;
END;
CREATE TRIGGER pause_moves_to_invalid_group AFTER UPDATE OF invalidated ON group_move_roots
WHEN OLD.invalidated = 0 AND NEW.invalidated = 1
BEGIN
    UPDATE model_move_scopes SET paused = 1 WHERE user_id = NEW.user_id AND birth_id IN
        (SELECT birth_id FROM model_move_frontier WHERE user_id = NEW.user_id AND target_birth_id = NEW.birth_id
         UNION SELECT model_birth_id FROM models WHERE model_user_id = NEW.user_id AND model_group_id IN
            (SELECT local_id FROM causal_genesis_bindings WHERE user_id = NEW.user_id AND entity_kind = 'group' AND birth_id = NEW.birth_id));
END;

-- Assignment changes are guarded by their monotonic revision, so the supported
-- transaction can refresh its guard. Other identity/lifecycle changes never do.
CREATE TRIGGER invalidate_model_move_domain AFTER UPDATE OF model_deleted, model_blob_id,
    model_unique_global_id, model_user_id, model_birth_id ON models
WHEN OLD.model_deleted IS NOT NEW.model_deleted OR OLD.model_blob_id IS NOT NEW.model_blob_id
    OR OLD.model_unique_global_id IS NOT NEW.model_unique_global_id
    OR OLD.model_user_id IS NOT NEW.model_user_id OR OLD.model_birth_id IS NOT NEW.model_birth_id
BEGIN
    UPDATE model_move_scopes SET invalidated = 1 WHERE user_id = OLD.model_user_id AND birth_id = OLD.model_birth_id;
END;
CREATE TRIGGER invalidate_deleted_model_move_scope BEFORE DELETE ON models
BEGIN
    UPDATE model_move_scopes SET invalidated = 1 WHERE user_id = OLD.model_user_id AND birth_id = OLD.model_birth_id;
END;
CREATE TRIGGER invalidate_model_move_blob AFTER UPDATE OF blob_sha256, blob_filetype, blob_size ON blobs
WHEN OLD.blob_sha256 IS NOT NEW.blob_sha256 OR OLD.blob_filetype IS NOT NEW.blob_filetype OR OLD.blob_size IS NOT NEW.blob_size
BEGIN
    UPDATE model_move_scopes SET invalidated = 1 WHERE (user_id, birth_id) IN
        (SELECT model_user_id, model_birth_id FROM models WHERE model_blob_id = NEW.blob_id);
END;
