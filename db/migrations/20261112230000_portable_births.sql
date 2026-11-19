ALTER TABLE models_group ADD COLUMN group_birth_id TEXT;
ALTER TABLE models ADD COLUMN model_birth_id TEXT;

CREATE TABLE entity_births (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    entity_kind TEXT NOT NULL CHECK (entity_kind IN ('group', 'model')),
    birth_id TEXT NOT NULL CHECK (length(birth_id) = 32 AND birth_id NOT GLOB '*[^0-9a-f]*'),
    global_id TEXT NOT NULL,
    PRIMARY KEY (user_id, entity_kind, birth_id)
);
CREATE UNIQUE INDEX group_live_birth ON models_group(group_user_id, group_birth_id) WHERE group_birth_id IS NOT NULL;
CREATE UNIQUE INDEX model_live_birth ON models(model_user_id, model_birth_id) WHERE model_birth_id IS NOT NULL;

CREATE TRIGGER register_group_birth AFTER INSERT ON models_group WHEN NEW.group_birth_id IS NOT NULL
BEGIN
    INSERT INTO entity_births(user_id, entity_kind, birth_id, global_id)
    VALUES (NEW.group_user_id, 'group', NEW.group_birth_id, NEW.group_unique_global_id);
END;
CREATE TRIGGER register_model_birth AFTER INSERT ON models WHEN NEW.model_birth_id IS NOT NULL
BEGIN
    INSERT INTO entity_births(user_id, entity_kind, birth_id, global_id)
    VALUES (NEW.model_user_id, 'model', NEW.model_birth_id, NEW.model_unique_global_id);
END;
CREATE TRIGGER immutable_group_birth BEFORE UPDATE OF group_birth_id ON models_group
WHEN OLD.group_birth_id IS NOT NEW.group_birth_id
BEGIN
    SELECT RAISE(ABORT, 'group birth is immutable');
END;
CREATE TRIGGER immutable_model_birth BEFORE UPDATE OF model_birth_id ON models
WHEN OLD.model_birth_id IS NOT NEW.model_birth_id
BEGIN
    SELECT RAISE(ABORT, 'model birth is immutable');
END;
CREATE TRIGGER rekey_group_birth AFTER UPDATE OF group_unique_global_id ON models_group
WHEN OLD.group_unique_global_id IS NOT NEW.group_unique_global_id AND NEW.group_birth_id IS NOT NULL
BEGIN
    UPDATE entity_births SET global_id = NEW.group_unique_global_id
    WHERE user_id = NEW.group_user_id AND entity_kind = 'group' AND birth_id = NEW.group_birth_id;
END;
CREATE TRIGGER rekey_model_birth AFTER UPDATE OF model_unique_global_id ON models
WHEN OLD.model_unique_global_id IS NOT NEW.model_unique_global_id AND NEW.model_birth_id IS NOT NULL
BEGIN
    UPDATE entity_births SET global_id = NEW.model_unique_global_id
    WHERE user_id = NEW.model_user_id AND entity_kind = 'model' AND birth_id = NEW.model_birth_id;
END;

