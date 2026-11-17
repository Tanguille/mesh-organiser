CREATE TABLE causal_replica_owners (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    replica_id TEXT NOT NULL,
    PRIMARY KEY (user_id, replica_id)
);

CREATE TABLE received_group_renames (
    replica_id TEXT NOT NULL,
    counter INTEGER NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    group_global_id TEXT NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (replica_id, counter),
    FOREIGN KEY (user_id, replica_id) REFERENCES causal_replica_owners(user_id, replica_id)
);

CREATE INDEX received_group_renames_owner_group ON received_group_renames(user_id, group_global_id);
CREATE TABLE received_group_rename_parents (
    replica_id TEXT NOT NULL,
    counter INTEGER NOT NULL,
    parent_replica_id TEXT NOT NULL,
    parent_counter INTEGER NOT NULL,
    PRIMARY KEY (replica_id, counter, parent_replica_id, parent_counter),
    FOREIGN KEY (replica_id, counter) REFERENCES received_group_renames(replica_id, counter) ON DELETE CASCADE
);
CREATE INDEX received_group_rename_parent_lookup ON received_group_rename_parents(parent_replica_id, parent_counter);
