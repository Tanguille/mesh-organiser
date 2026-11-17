-- Store each immutable envelope once. Creation and receipt rows retain only their
-- distinct provenance constraints; neither proves complete or applied ancestry.
CREATE TABLE causal_events (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    writer_id TEXT NOT NULL,
    counter INTEGER NOT NULL CHECK (counter > 0),
    envelope TEXT NOT NULL,
    PRIMARY KEY (writer_id, counter),
    UNIQUE (user_id, writer_id, counter)
);
CREATE INDEX causal_events_by_owner ON causal_events(user_id);
CREATE TRIGGER immutable_causal_event BEFORE UPDATE ON causal_events
BEGIN
    SELECT RAISE(ABORT, 'causal event is immutable');
END;
CREATE TRIGGER reject_legacy_causal_dot BEFORE INSERT ON causal_events
WHEN EXISTS (SELECT 1 FROM group_causal_events WHERE replica_id = NEW.writer_id AND counter = NEW.counter)
  OR EXISTS (SELECT 1 FROM membership_causal_events WHERE replica_id = NEW.writer_id AND counter = NEW.counter)
  OR EXISTS (SELECT 1 FROM received_group_renames WHERE replica_id = NEW.writer_id AND counter = NEW.counter)
BEGIN
    SELECT RAISE(ABORT, 'causal dot already exists in legacy history');
END;

-- Copy bytes without parsing or reserializing. Even an identical cross-store dot
-- collision aborts migration rather than guessing which provenance was intended.
INSERT INTO causal_events SELECT user_id, writer_id, counter, envelope FROM local_creation_records;
INSERT INTO causal_events SELECT user_id, writer_id, counter, envelope FROM causal_event_receipts;

CREATE TABLE local_creation_records_new (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    entity_kind TEXT NOT NULL CHECK (entity_kind IN ('group', 'model')),
    birth_id TEXT NOT NULL,
    writer_id TEXT NOT NULL,
    counter INTEGER NOT NULL CHECK (counter > 0),
    completeness TEXT NOT NULL DEFAULT 'incomplete' CHECK (completeness = 'incomplete'),
    PRIMARY KEY (user_id, entity_kind, birth_id),
    UNIQUE (writer_id, counter),
    FOREIGN KEY (user_id, entity_kind, birth_id) REFERENCES entity_births(user_id, entity_kind, birth_id),
    FOREIGN KEY (user_id, writer_id, counter) REFERENCES causal_events(user_id, writer_id, counter) ON DELETE CASCADE
);
INSERT INTO local_creation_records_new (user_id, entity_kind, birth_id, writer_id, counter)
SELECT user_id, entity_kind, birth_id, writer_id, counter FROM local_creation_records;

CREATE TABLE causal_event_receipts_new (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    writer_id TEXT NOT NULL,
    counter INTEGER NOT NULL CHECK (counter > 0),
    PRIMARY KEY (writer_id, counter),
    UNIQUE (user_id, writer_id, counter),
    FOREIGN KEY (user_id, writer_id) REFERENCES causal_replica_owners(user_id, replica_id),
    FOREIGN KEY (user_id, writer_id, counter) REFERENCES causal_events(user_id, writer_id, counter) ON DELETE CASCADE
);
INSERT INTO causal_event_receipts_new SELECT user_id, writer_id, counter FROM causal_event_receipts;

-- Remove referencing triggers/children explicitly before replacing the parents.
-- Rebuild edges against the final names; do not rely on ALTER TABLE rewriting FKs.
CREATE TABLE causal_dependencies_migration AS SELECT * FROM causal_receipt_dependencies;
DROP TABLE causal_receipt_dependencies;
DROP TRIGGER validate_entity_initialization_update;
DROP TABLE local_creation_records;
DROP TABLE causal_event_receipts;
ALTER TABLE local_creation_records_new RENAME TO local_creation_records;
ALTER TABLE causal_event_receipts_new RENAME TO causal_event_receipts;
CREATE INDEX causal_receipts_by_owner ON causal_event_receipts(user_id);

CREATE TABLE causal_receipt_dependencies (
    user_id INTEGER NOT NULL,
    writer_id TEXT NOT NULL,
    counter INTEGER NOT NULL,
    dependency_writer_id TEXT NOT NULL,
    dependency_counter INTEGER NOT NULL CHECK (dependency_counter > 0),
    PRIMARY KEY (writer_id, counter, dependency_writer_id, dependency_counter),
    FOREIGN KEY (user_id, writer_id, counter) REFERENCES causal_event_receipts(user_id, writer_id, counter) ON DELETE CASCADE
);
INSERT INTO causal_receipt_dependencies SELECT * FROM causal_dependencies_migration;
DROP TABLE causal_dependencies_migration;
CREATE INDEX causal_receipt_dependencies_by_owner ON causal_receipt_dependencies(user_id);

CREATE TRIGGER immutable_local_creation_record BEFORE UPDATE ON local_creation_records
BEGIN
    SELECT RAISE(ABORT, 'local creation record is immutable');
END;
CREATE TRIGGER immutable_causal_receipt BEFORE UPDATE ON causal_event_receipts
BEGIN
    SELECT RAISE(ABORT, 'causal receipt is immutable');
END;
CREATE TRIGGER distinct_local_creation_origin BEFORE INSERT ON local_creation_records
WHEN EXISTS (SELECT 1 FROM causal_event_receipts WHERE writer_id = NEW.writer_id AND counter = NEW.counter)
BEGIN
    SELECT RAISE(ABORT, 'causal event already has received origin');
END;
CREATE TRIGGER distinct_received_origin BEFORE INSERT ON causal_event_receipts
WHEN EXISTS (SELECT 1 FROM local_creation_records WHERE writer_id = NEW.writer_id AND counter = NEW.counter)
BEGIN
    SELECT RAISE(ABORT, 'causal event already has local origin');
END;

CREATE TRIGGER validate_entity_initialization_update BEFORE UPDATE OF initialization_status, initialization_writer_id, initialization_counter ON entity_births
WHEN NOT COALESCE((
    (NEW.initialization_status = 'unknown' AND NEW.initialization_writer_id IS NULL AND NEW.initialization_counter IS NULL)
    OR (NEW.initialization_status = 'created' AND EXISTS (
        SELECT 1 FROM local_creation_records c
        JOIN causal_events e ON e.user_id = c.user_id AND e.writer_id = c.writer_id AND e.counter = c.counter
        WHERE c.user_id = NEW.user_id AND c.entity_kind = NEW.entity_kind AND c.birth_id = NEW.birth_id
          AND c.writer_id = NEW.initialization_writer_id AND c.counter = NEW.initialization_counter
          AND json_extract(e.envelope, '$.entity.global_id') = NEW.global_id
    ))
), 0)
BEGIN
    SELECT RAISE(ABORT, 'invalid entity initialization');
END;

-- The initialization-insert and group/model rekey triggers still target entity_births;
-- preserve them, including rekey's reset to unknown. Only their evidence lookup changed.
CREATE INDEX causal_event_global_claim ON causal_events (
    user_id,
    json_extract(envelope, '$.entity.kind'),
    json_extract(envelope, '$.entity.global_id'),
    json_extract(envelope, '$.entity.birth_id')
);
CREATE INDEX causal_event_birth_claim ON causal_events (
    user_id,
    json_extract(envelope, '$.entity.kind'),
    json_extract(envelope, '$.entity.birth_id'),
    json_extract(envelope, '$.entity.global_id')
) WHERE json_extract(envelope, '$.entity.birth_id') IS NOT NULL;
