ALTER TABLE models_group ADD COLUMN group_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE models ADD COLUMN model_assignment_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE group_deletion_records ADD COLUMN group_revision INTEGER NOT NULL DEFAULT 0;

-- Triggers keep the compare-and-set revisions atomic across imports, APIs and recovery.
CREATE TRIGGER revise_model_assignment AFTER UPDATE OF model_group_id ON models
WHEN OLD.model_group_id IS NOT NEW.model_group_id
BEGIN
    UPDATE models SET model_assignment_revision = model_assignment_revision + 1 WHERE model_id = NEW.model_id;
    UPDATE models_group SET group_revision = group_revision + 1
    WHERE group_id IN (OLD.model_group_id, NEW.model_group_id);
END;

CREATE TRIGGER revise_group_metadata AFTER UPDATE OF group_name, group_deleted_at, group_resource_id, group_unique_global_id ON models_group
WHEN OLD.group_name IS NOT NEW.group_name OR OLD.group_deleted_at IS NOT NEW.group_deleted_at
  OR OLD.group_resource_id IS NOT NEW.group_resource_id OR OLD.group_unique_global_id IS NOT NEW.group_unique_global_id
BEGIN
    UPDATE models_group SET group_revision = group_revision + 1 WHERE group_id = NEW.group_id;
END;

CREATE TRIGGER revise_group_model_visibility AFTER UPDATE OF model_deleted ON models
WHEN OLD.model_deleted IS NOT NEW.model_deleted AND NEW.model_group_id IS NOT NULL
BEGIN
    UPDATE models_group SET group_revision = group_revision + 1 WHERE group_id = NEW.model_group_id;
END;

CREATE TRIGGER revise_group_model_removal AFTER DELETE ON models
WHEN OLD.model_group_id IS NOT NULL
BEGIN
    UPDATE models_group SET group_revision = group_revision + 1 WHERE group_id = OLD.model_group_id;
END;
