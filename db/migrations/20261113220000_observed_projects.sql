CREATE TABLE resource_observation_versions (
    resource_id INTEGER PRIMARY KEY REFERENCES resources(resource_id) ON DELETE CASCADE,
    revision INTEGER NOT NULL DEFAULT 0 CHECK(typeof(revision)='integer' AND revision>=0)
);
INSERT INTO resource_observation_versions(resource_id) SELECT resource_id FROM resources;
CREATE TRIGGER initialize_resource_observation AFTER INSERT ON resources
BEGIN INSERT INTO resource_observation_versions(resource_id) VALUES(NEW.resource_id); END;
CREATE TRIGGER revise_resource_observation AFTER UPDATE ON resources
WHEN OLD.resource_name IS NOT NEW.resource_name OR OLD.resource_flags IS NOT NEW.resource_flags
 OR OLD.resource_created IS NOT NEW.resource_created OR OLD.resource_last_modified IS NOT NEW.resource_last_modified
 OR OLD.resource_user_id IS NOT NEW.resource_user_id OR OLD.resource_unique_global_id IS NOT NEW.resource_unique_global_id
BEGIN UPDATE resource_observation_versions SET revision=revision+1 WHERE resource_id=NEW.resource_id; END;
ALTER TABLE causal_observation_batches ADD COLUMN project_snapshot TEXT CHECK(project_snapshot IS NULL OR json_valid(project_snapshot));
CREATE TABLE observed_project_bindings (
    user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
    source_instance_id TEXT NOT NULL,
    source_account_id TEXT NOT NULL,
    source_incarnation TEXT NOT NULL,
    global_id TEXT NOT NULL,
    local_resource_id INTEGER NOT NULL,
    local_incarnation TEXT NOT NULL,
    local_revision INTEGER NOT NULL,
    snapshot_digest TEXT NOT NULL,
    PRIMARY KEY(user_id,source_instance_id,source_account_id,source_incarnation)
);
CREATE TRIGGER immutable_observed_project_binding BEFORE UPDATE ON observed_project_bindings
BEGIN SELECT RAISE(ABORT,'observed project binding is immutable'); END;
ALTER TABLE group_birth_adoption_permits ADD COLUMN resource_id INTEGER;
ALTER TABLE group_birth_adoption_permits ADD COLUMN resource_incarnation TEXT;
ALTER TABLE group_birth_adoption_permits ADD COLUMN resource_revision INTEGER;
ALTER TABLE group_lifecycle_group_rows ADD COLUMN retained_resource_id INTEGER;
ALTER TABLE group_lifecycle_group_rows ADD COLUMN retained_resource_incarnation TEXT;
DROP TRIGGER pause_combined_group_purge;
CREATE TRIGGER pause_combined_group_purge BEFORE DELETE ON models_group
WHEN NOT EXISTS (
    SELECT 1 FROM group_lifecycle_group_rows r JOIN group_lifecycle_scopes s
        ON s.user_id=r.user_id AND s.entity_kind='group' AND s.birth_id=r.birth_id
    WHERE r.user_id=OLD.group_user_id AND r.birth_id=OLD.group_birth_id
      AND r.local_group_id=OLD.group_id AND r.state='purged' AND s.paused=0
      AND r.deletion_operation_id=OLD.group_deletion_operation_id
      AND OLD.group_deleted_at IS NOT NULL AND OLD.group_resource_id IS r.retained_resource_id
      AND (r.retained_resource_id IS NULL OR EXISTS(SELECT 1 FROM resources p WHERE p.resource_id=r.retained_resource_id AND p.resource_user_id=r.user_id AND p.resource_incarnation=r.retained_resource_incarnation))
)
BEGIN UPDATE group_lifecycle_scopes SET paused=1 WHERE user_id=OLD.group_user_id AND entity_kind='group' AND birth_id=OLD.group_birth_id; END;

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
          AND OLD.group_deleted_at IS NULL AND OLD.group_resource_id IS p.resource_id
          AND (p.resource_id IS NULL OR EXISTS(SELECT 1 FROM resources pr JOIN resource_observation_versions pv ON pv.resource_id=pr.resource_id WHERE pr.resource_id=p.resource_id AND pr.resource_user_id=p.user_id AND pr.resource_incarnation=p.resource_incarnation AND pv.revision=p.resource_revision))
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
