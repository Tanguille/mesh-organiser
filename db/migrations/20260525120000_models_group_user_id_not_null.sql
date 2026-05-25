-- no-transaction

-- SQLite cannot ALTER an existing column to NOT NULL; recreate the table instead.
-- models_group_new is temporary scaffolding and is renamed back to models_group below.

UPDATE models_group SET group_user_id = 1 WHERE group_user_id IS NULL;

PRAGMA foreign_keys=OFF;

CREATE TABLE models_group_new (
    group_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    group_name TEXT NOT NULL,
    group_created TEXT NOT NULL,
    group_unique_global_id TEXT NOT NULL,
    group_user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    group_last_modified TEXT NOT NULL,
    group_resource_id INTEGER NULL REFERENCES resources(resource_id) ON DELETE SET NULL
);

INSERT INTO models_group_new (
    group_id,
    group_name,
    group_created,
    group_unique_global_id,
    group_user_id,
    group_last_modified,
    group_resource_id
)
SELECT
    group_id,
    group_name,
    group_created,
    group_unique_global_id,
    group_user_id,
    group_last_modified,
    group_resource_id
FROM models_group;

DROP TABLE models_group;

ALTER TABLE models_group_new RENAME TO models_group;

CREATE INDEX IF NOT EXISTS idx_models_group_user_id ON models_group(group_user_id);

PRAGMA foreign_keys=ON;
