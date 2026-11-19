-- Observation epochs describe changes since capture, not pristine history.
CREATE TABLE group_observation_versions (
    group_id INTEGER PRIMARY KEY REFERENCES models_group(group_id) ON DELETE CASCADE,
    revision INTEGER NOT NULL DEFAULT 0 CHECK(typeof(revision)='integer' AND revision>=0)
);
INSERT INTO group_observation_versions(group_id) SELECT group_id FROM models_group;
CREATE TRIGGER initialize_group_observation AFTER INSERT ON models_group
BEGIN INSERT INTO group_observation_versions(group_id) VALUES(NEW.group_id); END;
CREATE TRIGGER revise_group_observation AFTER UPDATE ON models_group
WHEN OLD.group_name IS NOT NEW.group_name OR OLD.group_created IS NOT NEW.group_created
 OR OLD.group_last_modified IS NOT NEW.group_last_modified OR OLD.group_user_id IS NOT NEW.group_user_id
 OR OLD.group_unique_global_id IS NOT NEW.group_unique_global_id OR OLD.group_birth_id IS NOT NEW.group_birth_id
 OR OLD.group_resource_id IS NOT NEW.group_resource_id OR OLD.group_deleted_at IS NOT NEW.group_deleted_at
 OR OLD.group_deletion_operation_id IS NOT NEW.group_deletion_operation_id OR OLD.group_revision IS NOT NEW.group_revision
BEGIN UPDATE group_observation_versions SET revision=revision+1 WHERE group_id=NEW.group_id; END;
CREATE TRIGGER observe_group_member_insert AFTER INSERT ON models
WHEN NEW.model_group_id IS NOT NULL
BEGIN UPDATE group_observation_versions SET revision=revision+1 WHERE group_id=NEW.model_group_id; END;
CREATE TRIGGER observe_group_member_delete AFTER DELETE ON models
WHEN OLD.model_group_id IS NOT NULL
BEGIN UPDATE group_observation_versions SET revision=revision+1 WHERE group_id=OLD.model_group_id; END;

ALTER TABLE causal_genesis_bindings ADD COLUMN initialization_kind TEXT NOT NULL
    DEFAULT 'creation' CHECK(initialization_kind IN ('creation','observation'));
ALTER TABLE causal_genesis_bindings ADD COLUMN observation_evidence TEXT
    CHECK((initialization_kind='creation' AND observation_evidence IS NULL)
       OR (initialization_kind='observation' AND observation_evidence IS NOT NULL AND json_valid(observation_evidence)));
-- The local epoch includes birth assignment, so an exact source retry cannot
-- accidentally export a superseded snapshot after an otherwise invisible ABA.
ALTER TABLE causal_genesis_bindings ADD COLUMN local_observation_revision INTEGER
    CHECK(local_observation_revision IS NULL OR (typeof(local_observation_revision)='integer' AND local_observation_revision>=0));
CREATE UNIQUE INDEX local_group_observation_receipt ON causal_genesis_bindings
    (user_id,json_extract(observation_evidence,'$.observation_id'))
    WHERE initialization_kind='observation' AND origin='local';

-- A one-use local observation may name an unambiguous current legacy row.
-- It does not merge peer identities or turn its previous history into creation.
CREATE TABLE group_birth_adoption_permits (
    group_id INTEGER PRIMARY KEY REFERENCES models_group(group_id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
    account_id TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    global_id TEXT NOT NULL,
    birth_id TEXT NOT NULL,
    observation_revision INTEGER NOT NULL,
    head INTEGER NOT NULL
);
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
          AND NOT EXISTS(SELECT 1 FROM models WHERE model_group_id=OLD.group_id)
          AND NOT EXISTS(SELECT 1 FROM group_deletion_records WHERE user_id=p.user_id AND group_global_id=p.global_id)
          AND NOT EXISTS(SELECT 1 FROM entity_births WHERE user_id=p.user_id AND entity_kind='group' AND (global_id=p.global_id OR birth_id=p.birth_id))
    )
)
BEGIN SELECT RAISE(ABORT,'group birth is immutable'); END;
CREATE TRIGGER register_adopted_group_birth AFTER UPDATE OF group_birth_id ON models_group
WHEN OLD.group_birth_id IS NULL AND NEW.group_birth_id IS NOT NULL
BEGIN
    INSERT INTO entity_births(user_id,entity_kind,birth_id,global_id)
    VALUES(NEW.group_user_id,'group',NEW.group_birth_id,NEW.group_unique_global_id);
    UPDATE group_sync_heads SET identity_birth_id=NEW.group_birth_id
    WHERE user_id=NEW.group_user_id AND group_global_id=NEW.group_unique_global_id AND generation=1;
    DELETE FROM group_birth_adoption_permits WHERE group_id=NEW.group_id;
END;
