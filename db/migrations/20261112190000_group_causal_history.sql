CREATE TABLE group_causal_replica (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    replica_id TEXT NOT NULL,
    counter INTEGER NOT NULL
);
INSERT INTO group_causal_replica (singleton, replica_id, counter)
VALUES (1, lower(hex(randomblob(16))), 0);

CREATE TABLE group_causal_events (
    event_id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    group_global_id TEXT NOT NULL,
    replica_id TEXT NOT NULL,
    counter INTEGER NOT NULL,
    name TEXT NOT NULL,
    UNIQUE (replica_id, counter)
);
CREATE INDEX group_causal_events_by_group ON group_causal_events(user_id, group_global_id, event_id);

CREATE TABLE group_causal_parents (
    event_id INTEGER NOT NULL REFERENCES group_causal_events(event_id) ON DELETE CASCADE,
    parent_id INTEGER NOT NULL REFERENCES group_causal_events(event_id) ON DELETE CASCADE,
    PRIMARY KEY (event_id, parent_id)
);

CREATE TABLE group_causal_frontier (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    group_global_id TEXT NOT NULL,
    event_id INTEGER NOT NULL REFERENCES group_causal_events(event_id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, group_global_id, event_id)
);
