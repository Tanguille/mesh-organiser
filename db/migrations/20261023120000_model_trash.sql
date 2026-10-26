-- Soft delete: an RFC3339 UTC timestamp of when the model was moved to the trash, NULL while it is
-- not trashed. Trashed rows are purged once they are older than the configured retention.
ALTER TABLE models ADD model_deleted TEXT NULL;

CREATE INDEX IF NOT EXISTS idx_model_deleted ON models(model_deleted) WHERE model_deleted IS NOT NULL;
