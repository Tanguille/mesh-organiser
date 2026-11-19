CREATE TABLE unknown_model_identity_tombstones (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    model_global_id TEXT NOT NULL,
    PRIMARY KEY (user_id, model_global_id)
);

-- Purging a never-moved legacy model must not erase the last evidence of its unknown birth.
CREATE TRIGGER retain_unknown_model_identity BEFORE DELETE ON models
WHEN OLD.model_birth_id IS NULL AND EXISTS (SELECT 1 FROM users WHERE user_id = OLD.model_user_id)
BEGIN
    INSERT OR IGNORE INTO unknown_model_identity_tombstones (user_id, model_global_id)
    VALUES (OLD.model_user_id, OLD.model_unique_global_id);
END;

-- A real rekey also removes the live row's evidence for its previous global id.
CREATE TRIGGER retain_rekeyed_unknown_model_identity BEFORE UPDATE OF model_unique_global_id ON models
WHEN OLD.model_birth_id IS NULL AND OLD.model_unique_global_id IS NOT NEW.model_unique_global_id
BEGIN
    INSERT OR IGNORE INTO unknown_model_identity_tombstones (user_id, model_global_id)
    VALUES (OLD.model_user_id, OLD.model_unique_global_id);
END;
