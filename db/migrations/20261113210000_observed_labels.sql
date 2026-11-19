-- Label identity remains an observed source incarnation, not a global-id merge.
ALTER TABLE labels ADD COLUMN label_incarnation TEXT;
UPDATE labels SET label_incarnation=lower(hex(randomblob(16)));
CREATE UNIQUE INDEX label_incarnation ON labels(label_incarnation);
CREATE TRIGGER initialize_label_incarnation AFTER INSERT ON labels
BEGIN UPDATE labels SET label_incarnation=lower(hex(randomblob(16))) WHERE label_id=NEW.label_id; END;
CREATE TRIGGER immutable_label_incarnation BEFORE UPDATE OF label_incarnation ON labels
WHEN OLD.label_incarnation IS NOT NULL OR NEW.label_incarnation IS NULL
BEGIN SELECT RAISE(ABORT,'label incarnation is immutable'); END;
CREATE TABLE label_observation_versions (
    label_id INTEGER PRIMARY KEY REFERENCES labels(label_id) ON DELETE CASCADE,
    revision INTEGER NOT NULL DEFAULT 0 CHECK(typeof(revision)='integer' AND revision>=0)
);
INSERT INTO label_observation_versions(label_id) SELECT label_id FROM labels;
CREATE TRIGGER initialize_label_observation AFTER INSERT ON labels
BEGIN INSERT INTO label_observation_versions(label_id) VALUES(NEW.label_id); END;
CREATE TRIGGER revise_label_observation AFTER UPDATE ON labels
WHEN OLD.label_name IS NOT NEW.label_name OR OLD.label_color IS NOT NEW.label_color
 OR OLD.label_last_modified IS NOT NEW.label_last_modified OR OLD.label_user_id IS NOT NEW.label_user_id
 OR OLD.label_unique_global_id IS NOT NEW.label_unique_global_id
BEGIN UPDATE label_observation_versions SET revision=revision+1 WHERE label_id=NEW.label_id; END;
CREATE TRIGGER observe_label_edge_insert AFTER INSERT ON labels_labels
BEGIN UPDATE label_observation_versions SET revision=revision+1 WHERE label_id IN (NEW.parent_label_id,NEW.child_label_id); END;
CREATE TRIGGER observe_label_edge_delete AFTER DELETE ON labels_labels
BEGIN UPDATE label_observation_versions SET revision=revision+1 WHERE label_id IN (OLD.parent_label_id,OLD.child_label_id); END;
CREATE TRIGGER observe_label_edge_update AFTER UPDATE ON labels_labels
WHEN OLD.parent_label_id IS NOT NEW.parent_label_id OR OLD.child_label_id IS NOT NEW.child_label_id
BEGIN UPDATE label_observation_versions SET revision=revision+1 WHERE label_id IN (OLD.parent_label_id,OLD.child_label_id,NEW.parent_label_id,NEW.child_label_id); END;
ALTER TABLE causal_observation_batches ADD COLUMN label_snapshot TEXT CHECK(label_snapshot IS NULL OR json_valid(label_snapshot));
CREATE TABLE observed_label_bindings (
    user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
    source_instance_id TEXT NOT NULL,
    source_account_id TEXT NOT NULL,
    source_incarnation TEXT NOT NULL,
    global_id TEXT NOT NULL,
    local_label_id INTEGER NOT NULL,
    local_incarnation TEXT NOT NULL,
    local_revision INTEGER NOT NULL,
    snapshot_digest TEXT NOT NULL,
    component_digest TEXT NOT NULL,
    PRIMARY KEY(user_id,source_instance_id,source_account_id,source_incarnation)
);
CREATE INDEX observed_label_local_identity ON observed_label_bindings(user_id,local_incarnation);
CREATE TRIGGER immutable_observed_label_binding BEFORE UPDATE ON observed_label_bindings
BEGIN SELECT RAISE(ABORT,'observed label binding is immutable'); END;
-- Links are permitted only for the exact captured model epoch and link count.
ALTER TABLE model_birth_adoption_permits ADD COLUMN label_count INTEGER NOT NULL DEFAULT 0 CHECK(label_count>=0);

DROP TRIGGER immutable_model_birth;
CREATE TRIGGER immutable_model_birth BEFORE UPDATE OF model_birth_id ON models
WHEN OLD.model_birth_id IS NOT NEW.model_birth_id AND NOT (
    OLD.model_birth_id IS NULL AND NEW.model_birth_id IS NOT NULL AND EXISTS(
        SELECT 1 FROM model_birth_adoption_permits p
        JOIN sync_accounts a ON a.user_id=p.user_id AND a.account_id=p.account_id
        JOIN sync_instance i ON i.singleton=1 AND i.instance_id=p.instance_id
        JOIN model_observation_versions v ON v.model_id=p.model_id AND v.revision=p.observation_revision
        WHERE p.model_id=OLD.model_id AND p.user_id=OLD.model_user_id AND p.global_id=OLD.model_unique_global_id
          AND p.incarnation=OLD.model_incarnation AND p.original_birth_id IS OLD.model_birth_id AND p.birth_id=NEW.model_birth_id
          AND p.group_id=OLD.model_group_id AND OLD.model_deleted IS NULL
          AND (SELECT COUNT(*) FROM models_labels WHERE model_id=OLD.model_id)=p.label_count
          AND NOT EXISTS(SELECT 1 FROM entity_births WHERE user_id=p.user_id AND entity_kind='model' AND (global_id=p.global_id OR birth_id=p.birth_id))
    )
)
BEGIN SELECT RAISE(ABORT,'model birth is immutable'); END;
