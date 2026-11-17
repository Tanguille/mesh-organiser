ALTER TABLE group_causal_events ADD COLUMN event_kind TEXT NOT NULL DEFAULT 'rename';
ALTER TABLE group_causal_events ADD COLUMN deletion_operation_id TEXT;
ALTER TABLE group_causal_events ADD COLUMN deletion_replica_id TEXT;
ALTER TABLE group_causal_events ADD COLUMN deletion_counter INTEGER;
CREATE UNIQUE INDEX group_causal_deletion_event ON group_causal_events(user_id, group_global_id, deletion_operation_id) WHERE event_kind = 'delete';

CREATE TABLE membership_causal_events (
    event_id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    model_global_id TEXT NOT NULL,
    model_incarnation TEXT NOT NULL,
    replica_id TEXT NOT NULL,
    counter INTEGER NOT NULL,
    event_kind TEXT NOT NULL,
    group_global_id TEXT NOT NULL,
    deletion_operation_id TEXT NOT NULL,
    deletion_replica_id TEXT,
    deletion_counter INTEGER,
    UNIQUE(replica_id, counter)
);
CREATE INDEX membership_causal_events_by_model ON membership_causal_events(user_id, model_global_id, model_incarnation, event_id);
CREATE TABLE membership_causal_parents (
    event_id INTEGER NOT NULL REFERENCES membership_causal_events(event_id) ON DELETE CASCADE,
    parent_id INTEGER NOT NULL REFERENCES membership_causal_events(event_id) ON DELETE CASCADE,
    PRIMARY KEY(event_id, parent_id)
);
CREATE TABLE membership_causal_frontier (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    model_global_id TEXT NOT NULL,
    model_incarnation TEXT NOT NULL,
    event_id INTEGER NOT NULL REFERENCES membership_causal_events(event_id) ON DELETE CASCADE,
    PRIMARY KEY(user_id, model_global_id, model_incarnation, event_id)
);
