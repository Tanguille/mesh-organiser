-- A checked initial state for the narrow rename/membership scope, not a claim
-- that a legacy library or subsequent mutation history is complete.
CREATE TABLE causal_genesis_bindings (
    user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
    entity_kind TEXT NOT NULL CHECK (entity_kind IN ('group', 'model')),
    global_id TEXT NOT NULL,
    birth_id TEXT NOT NULL,
    writer_id TEXT NOT NULL,
    counter INTEGER NOT NULL,
    local_instance_id TEXT NOT NULL,
    local_account_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    source_account_id TEXT NOT NULL,
    local_id INTEGER NOT NULL,
    local_guard TEXT NOT NULL,
    materialization TEXT NOT NULL,
    origin TEXT NOT NULL CHECK (origin IN ('local', 'installed')),
    scope TEXT NOT NULL DEFAULT 'rename_move_genesis_v1' CHECK (scope = 'rename_move_genesis_v1'),
    PRIMARY KEY (user_id, entity_kind, birth_id),
    UNIQUE (user_id, entity_kind, global_id),
    FOREIGN KEY (user_id, writer_id, counter) REFERENCES causal_events(user_id, writer_id, counter) ON DELETE CASCADE
);
CREATE TRIGGER immutable_causal_genesis_binding BEFORE UPDATE ON causal_genesis_bindings
BEGIN
    SELECT RAISE(ABORT, 'genesis observation is immutable');
END;
