ALTER TABLE models_group ADD COLUMN group_deletion_operation_id TEXT;
ALTER TABLE group_sync_heads ADD COLUMN last_deletion_operation_id TEXT;
CREATE INDEX idx_group_owner_global_id ON models_group(group_user_id, group_unique_global_id);

-- Only associate legacy rows when their deletion history is unambiguous.
UPDATE models_group SET group_deletion_operation_id = CASE WHEN group_deleted_at IS NOT NULL THEN (
    SELECT MIN(operation_id) FROM group_deletion_records r
    WHERE r.user_id = models_group.group_user_id
      AND r.group_global_id = models_group.group_unique_global_id AND r.restored_at IS NULL
    HAVING COUNT(*) = 1
) ELSE (
    SELECT MIN(operation_id) FROM group_deletion_records r
    WHERE r.user_id = models_group.group_user_id
      AND r.group_global_id = models_group.group_unique_global_id
    HAVING COUNT(*) = 1
) END;
UPDATE group_sync_heads SET last_deletion_operation_id = COALESCE((
    SELECT MIN(group_deletion_operation_id) FROM models_group g
    WHERE g.group_user_id = group_sync_heads.user_id
      AND g.group_unique_global_id = group_sync_heads.group_global_id
    HAVING COUNT(*) = 1
), (
    SELECT MIN(operation_id) FROM group_deletion_records r
    WHERE r.user_id = group_sync_heads.user_id
      AND r.group_global_id = group_sync_heads.group_global_id AND r.restored_at IS NULL
    HAVING COUNT(*) = 1
), (
    SELECT MIN(operation_id) FROM group_deletion_records r
    WHERE r.user_id = group_sync_heads.user_id
      AND r.group_global_id = group_sync_heads.group_global_id
    HAVING COUNT(*) = 1
));
