-- Separation changes the current namespace generation without erasing its former owner evidence.
CREATE TABLE group_identity_separations (
 decision_id TEXT PRIMARY KEY REFERENCES sync_group_identity_choices(decision_id),
 user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 original_head TEXT NOT NULL CHECK(json_valid(original_head)),
 local_global_id TEXT NOT NULL,
 local_birth_id TEXT NOT NULL,
 remote_group_id INTEGER NOT NULL,
 remote_global_id TEXT NOT NULL,
 remote_birth_id TEXT NOT NULL,
 remote_generation INTEGER NOT NULL CHECK(remote_generation>0),
 remote_head INTEGER NOT NULL CHECK(remote_head>0)
);
CREATE TRIGGER immutable_group_identity_separation BEFORE UPDATE ON group_identity_separations
BEGIN SELECT RAISE(ABORT,'identity separation is immutable'); END;
