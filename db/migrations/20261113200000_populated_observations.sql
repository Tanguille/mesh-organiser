-- Mutable capture epochs cover edited models; pristine eligibility cannot detect
-- another edit once it is already false. These counters do not describe ancestry.
CREATE TABLE model_observation_versions (
    model_id INTEGER PRIMARY KEY REFERENCES models(model_id) ON DELETE CASCADE,
    revision INTEGER NOT NULL DEFAULT 0 CHECK(typeof(revision)='integer' AND revision>=0)
);
INSERT INTO model_observation_versions(model_id) SELECT model_id FROM models;
CREATE TRIGGER initialize_model_observation AFTER INSERT ON models
BEGIN INSERT INTO model_observation_versions(model_id) VALUES(NEW.model_id); END;
CREATE TRIGGER revise_model_observation AFTER UPDATE ON models
WHEN OLD.model_name IS NOT NEW.model_name OR OLD.model_url IS NOT NEW.model_url
 OR OLD.model_desc IS NOT NEW.model_desc OR OLD.model_flags IS NOT NEW.model_flags
 OR OLD.model_added IS NOT NEW.model_added OR OLD.model_last_modified IS NOT NEW.model_last_modified
 OR OLD.model_blob_id IS NOT NEW.model_blob_id OR OLD.model_group_id IS NOT NEW.model_group_id
 OR OLD.model_deleted IS NOT NEW.model_deleted OR OLD.model_birth_id IS NOT NEW.model_birth_id
 OR OLD.model_user_id IS NOT NEW.model_user_id OR OLD.model_unique_global_id IS NOT NEW.model_unique_global_id
BEGIN UPDATE model_observation_versions SET revision=revision+1 WHERE model_id=NEW.model_id; END;
CREATE TRIGGER observe_model_label_insert AFTER INSERT ON models_labels
BEGIN UPDATE model_observation_versions SET revision=revision+1 WHERE model_id=NEW.model_id; END;
CREATE TRIGGER observe_model_label_delete AFTER DELETE ON models_labels
BEGIN UPDATE model_observation_versions SET revision=revision+1 WHERE model_id=OLD.model_id; END;
CREATE TRIGGER observe_model_label_update AFTER UPDATE ON models_labels
WHEN OLD.model_id IS NOT NEW.model_id OR OLD.label_id IS NOT NEW.label_id
BEGIN UPDATE model_observation_versions SET revision=revision+1 WHERE model_id IN (OLD.model_id,NEW.model_id); END;
CREATE TRIGGER observe_model_blob AFTER UPDATE OF blob_sha256,blob_filetype,blob_size ON blobs
WHEN OLD.blob_sha256 IS NOT NEW.blob_sha256 OR OLD.blob_filetype IS NOT NEW.blob_filetype OR OLD.blob_size IS NOT NEW.blob_size
BEGIN UPDATE model_observation_versions SET revision=revision+1 WHERE model_id IN (SELECT model_id FROM models WHERE model_blob_id=NEW.blob_id); END;

CREATE TABLE causal_observation_batches (
    user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
    batch_id TEXT NOT NULL,
    local_instance_id TEXT NOT NULL,
    local_account_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    source_account_id TEXT NOT NULL,
    manifest TEXT NOT NULL CHECK(json_valid(manifest)),
    origin TEXT NOT NULL CHECK(origin IN ('local','installed')),
    PRIMARY KEY(user_id,batch_id)
);
CREATE TABLE causal_observation_batch_members (
    user_id INTEGER NOT NULL,
    batch_id TEXT NOT NULL,
    entity_kind TEXT NOT NULL,
    birth_id TEXT NOT NULL,
    PRIMARY KEY(user_id,entity_kind,birth_id),
    FOREIGN KEY(user_id,batch_id) REFERENCES causal_observation_batches(user_id,batch_id) ON DELETE CASCADE,
    FOREIGN KEY(user_id,entity_kind,birth_id) REFERENCES causal_genesis_bindings(user_id,entity_kind,birth_id) ON DELETE CASCADE
);
CREATE TRIGGER immutable_observation_batch BEFORE UPDATE ON causal_observation_batches
BEGIN SELECT RAISE(ABORT,'observation batch is immutable'); END;
CREATE TRIGGER immutable_observation_batch_member BEFORE UPDATE ON causal_observation_batch_members
BEGIN SELECT RAISE(ABORT,'observation membership is immutable'); END;

CREATE TABLE model_birth_adoption_permits (
    model_id INTEGER PRIMARY KEY REFERENCES models(model_id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
    account_id TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    global_id TEXT NOT NULL,
    incarnation TEXT NOT NULL,
    original_birth_id TEXT,
    birth_id TEXT NOT NULL,
    group_id INTEGER NOT NULL,
    observation_revision INTEGER NOT NULL
);
ALTER TABLE group_birth_adoption_permits ADD COLUMN member_count INTEGER NOT NULL DEFAULT 0 CHECK(member_count>=0);

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
          AND NOT EXISTS(SELECT 1 FROM models_labels WHERE model_id=OLD.model_id)
          AND NOT EXISTS(SELECT 1 FROM entity_births WHERE user_id=p.user_id AND entity_kind='model' AND (global_id=p.global_id OR birth_id=p.birth_id))
    )
)
BEGIN SELECT RAISE(ABORT,'model birth is immutable'); END;
CREATE TRIGGER register_adopted_model_birth AFTER UPDATE OF model_birth_id ON models
WHEN OLD.model_birth_id IS NULL AND NEW.model_birth_id IS NOT NULL
BEGIN
    INSERT INTO entity_births(user_id,entity_kind,birth_id,global_id)
    VALUES(NEW.model_user_id,'model',NEW.model_birth_id,NEW.model_unique_global_id);
    DELETE FROM model_birth_adoption_permits WHERE model_id=NEW.model_id;
END;

-- This maps one proven local incarnation, not every row sharing a global id.
-- Old journal bytes and their unknown ancestry remain untouched.
CREATE TABLE model_identity_adoptions (
    user_id INTEGER NOT NULL,
    entity_kind TEXT NOT NULL DEFAULT 'model' CHECK(entity_kind='model'),
    birth_id TEXT NOT NULL,
    model_global_id TEXT NOT NULL,
    model_incarnation TEXT NOT NULL,
    PRIMARY KEY(user_id,model_incarnation),
    FOREIGN KEY(user_id,entity_kind,birth_id) REFERENCES causal_genesis_bindings(user_id,entity_kind,birth_id) ON DELETE CASCADE
);
CREATE TRIGGER immutable_model_identity_adoption BEFORE UPDATE ON model_identity_adoptions
BEGIN SELECT RAISE(ABORT,'model identity adoption is immutable'); END;

DROP TRIGGER immutable_group_birth;
CREATE TRIGGER immutable_group_birth BEFORE UPDATE OF group_birth_id ON models_group
WHEN OLD.group_birth_id IS NOT NEW.group_birth_id AND NOT (
    OLD.group_birth_id IS NULL AND NEW.group_birth_id IS NOT NULL AND EXISTS (
        SELECT 1 FROM group_birth_adoption_permits p
        JOIN sync_accounts a ON a.user_id=p.user_id AND a.account_id=p.account_id
        JOIN sync_instance i ON i.singleton=1 AND i.instance_id=p.instance_id
        JOIN group_observation_versions v ON v.group_id=p.group_id AND v.revision=p.observation_revision
        JOIN group_sync_heads h ON h.user_id=p.user_id AND h.group_global_id=p.global_id
        WHERE p.group_id=OLD.group_id AND p.user_id=OLD.group_user_id AND p.global_id=OLD.group_unique_global_id
          AND p.birth_id=NEW.group_birth_id AND h.generation=1 AND h.head=p.head AND h.identity_birth_id IS NULL
          AND OLD.group_deleted_at IS NULL AND OLD.group_resource_id IS NULL
          AND (SELECT COUNT(*) FROM models WHERE model_group_id=OLD.group_id)=p.member_count
          AND NOT EXISTS(SELECT 1 FROM models m WHERE m.model_group_id=OLD.group_id AND NOT EXISTS(
              SELECT 1 FROM model_birth_adoption_permits mp JOIN model_observation_versions mv ON mv.model_id=mp.model_id
              WHERE mp.model_id=m.model_id AND mp.user_id=p.user_id AND mp.account_id=p.account_id AND mp.instance_id=p.instance_id
                AND mp.group_id=OLD.group_id AND mp.global_id=m.model_unique_global_id AND mp.incarnation=m.model_incarnation
                AND mp.original_birth_id IS m.model_birth_id AND mp.observation_revision=mv.revision
          ))
          AND NOT EXISTS(SELECT 1 FROM group_deletion_records WHERE user_id=p.user_id AND group_global_id=p.global_id)
          AND NOT EXISTS(SELECT 1 FROM entity_births WHERE user_id=p.user_id AND entity_kind='group' AND (global_id=p.global_id OR birth_id=p.birth_id))
    )
)
BEGIN SELECT RAISE(ABORT,'group birth is immutable'); END;
