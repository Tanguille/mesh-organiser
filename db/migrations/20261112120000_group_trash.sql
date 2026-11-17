ALTER TABLE models_group ADD COLUMN group_deleted_at TEXT NULL;

CREATE TABLE group_recovery_members (
    group_id INTEGER NOT NULL REFERENCES models_group(group_id) ON DELETE CASCADE,
    model_id INTEGER NOT NULL REFERENCES models(model_id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, model_id)
);

CREATE TABLE group_deletion_records (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    group_global_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    deleted_at TEXT NOT NULL,
    restored_at TEXT NULL,
    PRIMARY KEY (user_id, operation_id)
);

CREATE INDEX idx_group_trash ON models_group(group_user_id, group_deleted_at);
