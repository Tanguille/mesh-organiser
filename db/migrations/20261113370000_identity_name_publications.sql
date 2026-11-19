-- Keep an exact accepted request after its outbox is acknowledged or discarded.
CREATE TABLE group_identity_name_publications (
 user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 decision_id TEXT NOT NULL REFERENCES sync_group_identity_choices(decision_id) ON DELETE CASCADE,
 request_id TEXT NOT NULL,
 request TEXT NOT NULL,
 writer_id TEXT NOT NULL,
 counter INTEGER NOT NULL,
 outbox_id TEXT NOT NULL,
 PRIMARY KEY(user_id,decision_id,request_id),
 FOREIGN KEY(user_id,writer_id,counter) REFERENCES causal_events(user_id,writer_id,counter)
);
CREATE TRIGGER immutable_identity_name_publication BEFORE UPDATE ON group_identity_name_publications
BEGIN SELECT RAISE(ABORT,'identity name publication is immutable'); END;
