ALTER TABLE models ADD COLUMN model_incarnation TEXT NOT NULL DEFAULT '';

UPDATE models SET model_incarnation = LOWER(HEX(RANDOMBLOB(16)));
CREATE UNIQUE INDEX idx_model_incarnation ON models(model_incarnation);

-- SQLite cannot add a column with a random default, so initialize it inside the insert.
CREATE TRIGGER initialize_model_incarnation AFTER INSERT ON models
BEGIN
    UPDATE models SET model_incarnation = LOWER(HEX(RANDOMBLOB(16))) WHERE model_id = NEW.model_id;
END;

CREATE TRIGGER immutable_model_incarnation BEFORE UPDATE OF model_incarnation ON models
WHEN OLD.model_incarnation != '' OR NEW.model_incarnation = ''
BEGIN
    SELECT RAISE(ABORT, 'model incarnation is immutable');
END;
