ALTER TABLE causal_events ADD COLUMN encoding TEXT NOT NULL DEFAULT 'portable_v2'
    CHECK (encoding IN ('portable_v2', 'local_history_v1'));

-- Local edits are not received identity claims. Keep their unbounded history out
-- of the indexes used by bounded receipt inspection.
DROP INDEX causal_event_global_claim;
DROP INDEX causal_event_birth_claim;
CREATE INDEX causal_event_global_claim ON causal_events (
    user_id,
    json_extract(envelope, '$.entity.kind'),
    json_extract(envelope, '$.entity.global_id'),
    json_extract(envelope, '$.entity.birth_id')
) WHERE encoding = 'portable_v2';
CREATE INDEX causal_event_birth_claim ON causal_events (
    user_id,
    json_extract(envelope, '$.entity.kind'),
    json_extract(envelope, '$.entity.birth_id'),
    json_extract(envelope, '$.entity.global_id')
) WHERE encoding = 'portable_v2' AND json_extract(envelope, '$.entity.birth_id') IS NOT NULL;

-- Local histories now share the dot namespace. Only the old remote rename inbox
-- remains outside it; local-only facts never become portable events by migration.
DROP TRIGGER reject_legacy_causal_dot;
CREATE TRIGGER reject_legacy_causal_dot BEFORE INSERT ON causal_events
WHEN EXISTS (SELECT 1 FROM received_group_renames WHERE replica_id = NEW.writer_id AND counter = NEW.counter)
BEGIN
    SELECT RAISE(ABORT, 'causal dot already exists in legacy inbox');
END;

INSERT INTO causal_events (user_id, writer_id, counter, encoding, envelope)
SELECT user_id, replica_id, counter, 'local_history_v1', json_object(
    'entity', json_object('kind', 'group', 'global_id', group_global_id,
        'birth_id', NULL, 'local_incarnation', NULL),
    'ancestry', 'unknown',
    'action', json_object('kind', event_kind, 'name', name,
        'deletion_operation_id', deletion_operation_id, 'deletion_replica_id', deletion_replica_id,
        'deletion_counter', deletion_counter, 'model_global_id', model_global_id,
        'model_incarnation', model_incarnation, 'membership_replica_id', membership_replica_id,
        'membership_counter', membership_counter)
) FROM group_causal_events;

INSERT INTO causal_events (user_id, writer_id, counter, encoding, envelope)
SELECT user_id, replica_id, counter, 'local_history_v1', json_object(
    'entity', json_object('kind', 'model', 'global_id', model_global_id,
        'birth_id', model_birth_id, 'local_incarnation', model_incarnation),
    'ancestry', 'unknown',
    'action', json_object('kind', event_kind, 'group_global_id', group_global_id,
        'deletion_operation_id', deletion_operation_id, 'deletion_replica_id', deletion_replica_id,
        'deletion_counter', deletion_counter, 'from_group_global_id', from_group_global_id,
        'to_group_global_id', to_group_global_id)
) FROM membership_causal_events;

CREATE TABLE group_causal_events_new (
    event_id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    group_global_id TEXT NOT NULL,
    replica_id TEXT NOT NULL,
    counter INTEGER NOT NULL,
    UNIQUE (replica_id, counter),
    FOREIGN KEY (user_id, replica_id, counter) REFERENCES causal_events(user_id, writer_id, counter) ON DELETE CASCADE
);
INSERT INTO group_causal_events_new
SELECT event_id, user_id, group_global_id, replica_id, counter FROM group_causal_events;

CREATE TABLE membership_causal_events_new (
    event_id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    model_global_id TEXT NOT NULL,
    model_incarnation TEXT NOT NULL,
    replica_id TEXT NOT NULL,
    counter INTEGER NOT NULL,
    model_birth_id TEXT,
    UNIQUE (replica_id, counter),
    FOREIGN KEY (user_id, replica_id, counter) REFERENCES causal_events(user_id, writer_id, counter) ON DELETE CASCADE
);
INSERT INTO membership_causal_events_new
SELECT event_id, user_id, model_global_id, model_incarnation, replica_id, counter, model_birth_id FROM membership_causal_events;

-- Keep the exact recorded edges and tips. In particular, never attach old roots
-- to isolated creation records just because the current global ids match.
CREATE TABLE group_parents_migration AS SELECT * FROM group_causal_parents;
CREATE TABLE group_frontier_migration AS SELECT * FROM group_causal_frontier;
CREATE TABLE membership_parents_migration AS SELECT * FROM membership_causal_parents;
CREATE TABLE membership_frontier_migration AS SELECT * FROM membership_causal_frontier;
DROP TABLE group_causal_parents;
DROP TABLE group_causal_frontier;
DROP TABLE membership_causal_parents;
DROP TABLE membership_causal_frontier;
DROP TRIGGER capture_membership_birth;
DROP TABLE group_causal_events;
DROP TABLE membership_causal_events;
ALTER TABLE group_causal_events_new RENAME TO group_causal_events;
ALTER TABLE membership_causal_events_new RENAME TO membership_causal_events;

CREATE INDEX group_causal_events_by_group ON group_causal_events(user_id, group_global_id, event_id);
CREATE INDEX membership_causal_events_by_model ON membership_causal_events(user_id, model_global_id, model_incarnation, event_id);
CREATE INDEX membership_birth_evidence ON membership_causal_events(user_id, model_global_id, model_birth_id);
CREATE UNIQUE INDEX group_causal_deletion_event ON causal_events(
    user_id, json_extract(envelope, '$.entity.global_id'), json_extract(envelope, '$.action.deletion_operation_id')
) WHERE encoding = 'local_history_v1' AND json_extract(envelope, '$.entity.kind') = 'group'
    AND json_extract(envelope, '$.action.kind') = 'delete';

CREATE TABLE group_causal_parents (
    event_id INTEGER NOT NULL REFERENCES group_causal_events(event_id) ON DELETE CASCADE,
    parent_id INTEGER NOT NULL REFERENCES group_causal_events(event_id) ON DELETE CASCADE,
    PRIMARY KEY (event_id, parent_id)
);
INSERT INTO group_causal_parents SELECT * FROM group_parents_migration;
DROP TABLE group_parents_migration;
CREATE TABLE group_causal_frontier (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    group_global_id TEXT NOT NULL,
    event_id INTEGER NOT NULL REFERENCES group_causal_events(event_id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, group_global_id, event_id)
);
INSERT INTO group_causal_frontier SELECT * FROM group_frontier_migration;
DROP TABLE group_frontier_migration;
CREATE TABLE membership_causal_parents (
    event_id INTEGER NOT NULL REFERENCES membership_causal_events(event_id) ON DELETE CASCADE,
    parent_id INTEGER NOT NULL REFERENCES membership_causal_events(event_id) ON DELETE CASCADE,
    PRIMARY KEY (event_id, parent_id)
);
INSERT INTO membership_causal_parents SELECT * FROM membership_parents_migration;
DROP TABLE membership_parents_migration;
CREATE TABLE membership_causal_frontier (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    model_global_id TEXT NOT NULL,
    model_incarnation TEXT NOT NULL,
    event_id INTEGER NOT NULL REFERENCES membership_causal_events(event_id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, model_global_id, model_incarnation, event_id)
);
INSERT INTO membership_causal_frontier SELECT * FROM membership_frontier_migration;
DROP TABLE membership_frontier_migration;

-- These projections preserve the local reader shapes without another payload copy.
CREATE VIEW group_causal_event_facts AS SELECT m.*,
    json_extract(e.envelope, '$.action.kind') AS event_kind,
    json_extract(e.envelope, '$.action.name') AS name,
    json_extract(e.envelope, '$.action.deletion_operation_id') AS deletion_operation_id,
    json_extract(e.envelope, '$.action.deletion_replica_id') AS deletion_replica_id,
    json_extract(e.envelope, '$.action.deletion_counter') AS deletion_counter,
    json_extract(e.envelope, '$.action.model_global_id') AS model_global_id,
    json_extract(e.envelope, '$.action.model_incarnation') AS model_incarnation,
    json_extract(e.envelope, '$.action.membership_replica_id') AS membership_replica_id,
    json_extract(e.envelope, '$.action.membership_counter') AS membership_counter
FROM group_causal_events m JOIN causal_events e ON e.user_id = m.user_id AND e.writer_id = m.replica_id AND e.counter = m.counter
WHERE e.encoding = 'local_history_v1';
CREATE VIEW membership_causal_event_facts AS SELECT m.*,
    json_extract(e.envelope, '$.action.kind') AS event_kind,
    json_extract(e.envelope, '$.action.group_global_id') AS group_global_id,
    json_extract(e.envelope, '$.action.deletion_operation_id') AS deletion_operation_id,
    json_extract(e.envelope, '$.action.deletion_replica_id') AS deletion_replica_id,
    json_extract(e.envelope, '$.action.deletion_counter') AS deletion_counter,
    json_extract(e.envelope, '$.action.from_group_global_id') AS from_group_global_id,
    json_extract(e.envelope, '$.action.to_group_global_id') AS to_group_global_id
FROM membership_causal_events m JOIN causal_events e ON e.user_id = m.user_id AND e.writer_id = m.replica_id AND e.counter = m.counter
WHERE e.encoding = 'local_history_v1';

CREATE TRIGGER immutable_group_causal_metadata BEFORE UPDATE ON group_causal_events
BEGIN
    SELECT RAISE(ABORT, 'local group event metadata is immutable');
END;
CREATE TRIGGER immutable_membership_causal_metadata BEFORE UPDATE ON membership_causal_events
BEGIN
    SELECT RAISE(ABORT, 'local membership event metadata is immutable');
END;
CREATE TRIGGER validate_group_causal_metadata BEFORE INSERT ON group_causal_events
WHEN NOT EXISTS (SELECT 1 FROM causal_events e WHERE e.user_id = NEW.user_id AND e.writer_id = NEW.replica_id AND e.counter = NEW.counter
    AND e.encoding = 'local_history_v1' AND json_extract(e.envelope, '$.entity.kind') = 'group'
    AND json_extract(e.envelope, '$.entity.global_id') = NEW.group_global_id)
BEGIN
    SELECT RAISE(ABORT, 'invalid local group event metadata');
END;
CREATE TRIGGER validate_membership_causal_metadata BEFORE INSERT ON membership_causal_events
WHEN NOT EXISTS (SELECT 1 FROM causal_events e WHERE e.user_id = NEW.user_id AND e.writer_id = NEW.replica_id AND e.counter = NEW.counter
    AND e.encoding = 'local_history_v1' AND json_extract(e.envelope, '$.entity.kind') = 'model'
    AND json_extract(e.envelope, '$.entity.global_id') = NEW.model_global_id
    AND json_extract(e.envelope, '$.entity.local_incarnation') = NEW.model_incarnation
    AND json_extract(e.envelope, '$.entity.birth_id') IS NEW.model_birth_id)
BEGIN
    SELECT RAISE(ABORT, 'invalid local membership event metadata');
END;

CREATE TRIGGER portable_creation_encoding BEFORE INSERT ON local_creation_records
WHEN NOT EXISTS (SELECT 1 FROM causal_events WHERE user_id = NEW.user_id AND writer_id = NEW.writer_id AND counter = NEW.counter AND encoding = 'portable_v2')
BEGIN
    SELECT RAISE(ABORT, 'creation requires a portable envelope');
END;
CREATE TRIGGER portable_receipt_encoding BEFORE INSERT ON causal_event_receipts
WHEN NOT EXISTS (SELECT 1 FROM causal_events WHERE user_id = NEW.user_id AND writer_id = NEW.writer_id AND counter = NEW.counter AND encoding = 'portable_v2')
BEGIN
    SELECT RAISE(ABORT, 'receipt requires a portable envelope');
END;
