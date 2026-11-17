ALTER TABLE models ADD COLUMN model_lifecycle_revision INTEGER NOT NULL DEFAULT 0;

CREATE TRIGGER revise_model_lifecycle AFTER UPDATE OF model_deleted ON models
WHEN (OLD.model_deleted IS NULL) != (NEW.model_deleted IS NULL)
BEGIN
    SELECT CASE WHEN OLD.model_lifecycle_revision = 9223372036854775807
        THEN RAISE(ABORT, 'model lifecycle revision exhausted') END;
    UPDATE models SET model_lifecycle_revision = OLD.model_lifecycle_revision + 1
    WHERE model_id = NEW.model_id;
END;
