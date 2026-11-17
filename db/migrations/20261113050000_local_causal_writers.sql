CREATE TABLE local_causal_writers (
    replica_id TEXT PRIMARY KEY
);

INSERT OR IGNORE INTO local_causal_writers (replica_id)
SELECT replica_id FROM group_causal_replica
UNION SELECT replica_id FROM group_causal_events
UNION SELECT replica_id FROM membership_causal_events
UNION SELECT writer_id FROM local_creation_records;
