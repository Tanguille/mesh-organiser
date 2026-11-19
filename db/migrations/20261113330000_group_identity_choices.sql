-- A confirmed match adds evidence; it never rewrites either observed history.
CREATE TABLE group_identity_observations (
 user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 observation_id TEXT NOT NULL,
 birth_id TEXT NOT NULL,
 writer_id TEXT NOT NULL,
 counter INTEGER NOT NULL,
 item TEXT NOT NULL,
 PRIMARY KEY(user_id,observation_id),
 UNIQUE(user_id,writer_id,counter),
 FOREIGN KEY(user_id,writer_id,counter) REFERENCES causal_events(user_id,writer_id,counter)
);
CREATE TRIGGER immutable_group_identity_observation BEFORE UPDATE ON group_identity_observations
BEGIN SELECT RAISE(ABORT,'identity observation is immutable'); END;
CREATE TABLE sync_group_identity_choices (
 decision_id TEXT PRIMARY KEY,
 user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 local_group_id INTEGER NOT NULL,
 authorization TEXT NOT NULL,
 delivery TEXT NOT NULL,
 item TEXT NOT NULL,
 snapshot TEXT NOT NULL,
 writer_id TEXT,
 counter INTEGER,
 CHECK((writer_id IS NULL)=(counter IS NULL))
);
CREATE TRIGGER immutable_confirmed_group_identity_choice BEFORE UPDATE ON sync_group_identity_choices
WHEN OLD.writer_id IS NOT NULL
BEGIN SELECT RAISE(ABORT,'confirmed identity choice is immutable'); END;
CREATE TABLE sync_transport_log_new (
 user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 sequence INTEGER NOT NULL CHECK(sequence>0),
 kind TEXT NOT NULL CHECK(kind IN ('group_genesis','group_operation','observed_group','group_identity_observation')),
 item_id TEXT NOT NULL,digest TEXT NOT NULL,
 authority_writer_id TEXT NOT NULL REFERENCES sync_transport_writers(writer_id),
 writer_id TEXT NOT NULL,
 PRIMARY KEY(user_id,sequence),UNIQUE(user_id,kind,item_id)
);
INSERT INTO sync_transport_log_new SELECT * FROM sync_transport_log;
DROP TABLE sync_transport_log;
ALTER TABLE sync_transport_log_new RENAME TO sync_transport_log;
CREATE TRIGGER immutable_sync_transport_log BEFORE UPDATE ON sync_transport_log
BEGIN SELECT RAISE(ABORT,'transport receipt is immutable'); END;
