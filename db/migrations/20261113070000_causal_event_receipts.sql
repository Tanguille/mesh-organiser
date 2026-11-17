CREATE TABLE causal_event_receipts (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    writer_id TEXT NOT NULL,
    counter INTEGER NOT NULL CHECK (counter > 0),
    envelope TEXT NOT NULL,
    PRIMARY KEY (writer_id, counter),
    UNIQUE (user_id, writer_id, counter),
    FOREIGN KEY (user_id, writer_id) REFERENCES causal_replica_owners(user_id, replica_id)
);
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
CREATE INDEX causal_receipt_dependencies_by_owner ON causal_receipt_dependencies(user_id);

CREATE TRIGGER immutable_causal_receipt BEFORE UPDATE ON causal_event_receipts
BEGIN
    SELECT RAISE(ABORT, 'causal receipt is immutable');
END;
