ALTER TABLE entity_births ADD COLUMN initialization_status TEXT NOT NULL DEFAULT 'unknown';
ALTER TABLE entity_births ADD COLUMN initialization_writer_id TEXT;
ALTER TABLE entity_births ADD COLUMN initialization_counter INTEGER;

CREATE TRIGGER validate_entity_initialization_insert BEFORE INSERT ON entity_births
WHEN NEW.initialization_status != 'unknown' OR NEW.initialization_writer_id IS NOT NULL OR NEW.initialization_counter IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invalid entity initialization');
END;

CREATE TRIGGER validate_entity_initialization_update BEFORE UPDATE OF initialization_status, initialization_writer_id, initialization_counter ON entity_births
WHEN NOT COALESCE((
    (NEW.initialization_status = 'unknown' AND NEW.initialization_writer_id IS NULL AND NEW.initialization_counter IS NULL)
    OR (NEW.initialization_status = 'created' AND EXISTS (
        SELECT 1 FROM local_creation_records c
        WHERE c.user_id = NEW.user_id AND c.entity_kind = NEW.entity_kind AND c.birth_id = NEW.birth_id
          AND c.writer_id = NEW.initialization_writer_id AND c.counter = NEW.initialization_counter
          AND json_extract(c.envelope, '$.entity.global_id') = NEW.global_id
    ))
), 0)
BEGIN
    SELECT RAISE(ABORT, 'invalid entity initialization');
END;

DROP TRIGGER rekey_group_birth;
CREATE TRIGGER rekey_group_birth AFTER UPDATE OF group_unique_global_id ON models_group
WHEN OLD.group_unique_global_id IS NOT NEW.group_unique_global_id AND NEW.group_birth_id IS NOT NULL
BEGIN
    UPDATE entity_births SET global_id = NEW.group_unique_global_id, initialization_status = 'unknown', initialization_writer_id = NULL, initialization_counter = NULL
    WHERE user_id = NEW.group_user_id AND entity_kind = 'group' AND birth_id = NEW.group_birth_id;
END;

DROP TRIGGER rekey_model_birth;
CREATE TRIGGER rekey_model_birth AFTER UPDATE OF model_unique_global_id ON models
WHEN OLD.model_unique_global_id IS NOT NEW.model_unique_global_id AND NEW.model_birth_id IS NOT NULL
BEGIN
    UPDATE entity_births SET global_id = NEW.model_unique_global_id, initialization_status = 'unknown', initialization_writer_id = NULL, initialization_counter = NULL
    WHERE user_id = NEW.model_user_id AND entity_kind = 'model' AND birth_id = NEW.model_birth_id;
END;
