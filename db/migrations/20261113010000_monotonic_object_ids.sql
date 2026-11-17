CREATE TABLE object_id_highwater (
    object_kind TEXT PRIMARY KEY CHECK (object_kind IN ('model', 'resource')),
    highwater INTEGER NOT NULL CHECK (typeof(highwater) = 'integer' AND highwater >= 0)
);
INSERT INTO object_id_highwater (object_kind, highwater)
SELECT 'model', max(COALESCE(MAX(model_id), 0), 0) FROM models;
INSERT INTO object_id_highwater (object_kind, highwater)
SELECT 'resource', max(COALESCE(MAX(resource_id), 0), 0) FROM resources;

CREATE TRIGGER model_monotonic_id AFTER INSERT ON models
BEGIN
    SELECT CASE WHEN NOT EXISTS (SELECT 1 FROM object_id_highwater WHERE object_kind = 'model' AND NEW.model_id > highwater)
        THEN RAISE(ABORT, 'model id must increase') END;
    UPDATE object_id_highwater SET highwater = NEW.model_id WHERE object_kind = 'model';
END;
CREATE TRIGGER resource_monotonic_id AFTER INSERT ON resources
BEGIN
    SELECT CASE WHEN NOT EXISTS (SELECT 1 FROM object_id_highwater WHERE object_kind = 'resource' AND NEW.resource_id > highwater)
        THEN RAISE(ABORT, 'resource id must increase') END;
    UPDATE object_id_highwater SET highwater = NEW.resource_id WHERE object_kind = 'resource';
END;
CREATE TRIGGER model_immutable_id BEFORE UPDATE ON models
WHEN OLD.model_id IS NOT NEW.model_id
BEGIN
    SELECT RAISE(ABORT, 'model id is immutable');
END;
CREATE TRIGGER resource_immutable_id BEFORE UPDATE ON resources
WHEN OLD.resource_id IS NOT NEW.resource_id
BEGIN
    SELECT RAISE(ABORT, 'resource id is immutable');
END;
