-- ModelCreated did not capture metadata. Existing rows cannot prove that their
-- current values are initial values, even when timestamps/revisions still match.
ALTER TABLE models ADD COLUMN model_genesis_unchanged INTEGER NOT NULL DEFAULT 0
    CHECK (model_genesis_unchanged IN (0, 1));

-- Only the actual local creation INSERT may opt into this proof. Once lost, a
-- value restored to its initial contents cannot silently regain eligibility.
CREATE TRIGGER irreversible_model_genesis_eligibility
BEFORE UPDATE OF model_genesis_unchanged ON models
WHEN NEW.model_genesis_unchanged > OLD.model_genesis_unchanged
BEGIN
    SELECT RAISE(ABORT, 'model genesis eligibility cannot be restored');
END;

CREATE TRIGGER invalidate_model_genesis_metadata
AFTER UPDATE OF model_name, model_blob_id, model_url, model_desc, model_flags,
    model_added, model_last_modified, model_user_id, model_unique_global_id,
    model_birth_id, model_group_id, model_deleted ON models
WHEN OLD.model_name IS NOT NEW.model_name
    OR OLD.model_blob_id IS NOT NEW.model_blob_id
    OR OLD.model_url IS NOT NEW.model_url
    OR OLD.model_desc IS NOT NEW.model_desc
    OR OLD.model_flags IS NOT NEW.model_flags
    OR OLD.model_added IS NOT NEW.model_added
    OR OLD.model_last_modified IS NOT NEW.model_last_modified
    OR OLD.model_user_id IS NOT NEW.model_user_id
    OR OLD.model_unique_global_id IS NOT NEW.model_unique_global_id
    OR OLD.model_birth_id IS NOT NEW.model_birth_id
    OR OLD.model_group_id IS NOT NEW.model_group_id
    OR OLD.model_deleted IS NOT NEW.model_deleted
BEGIN
    UPDATE models SET model_genesis_unchanged = 0 WHERE model_id = NEW.model_id;
END;

CREATE TRIGGER invalidate_model_genesis_label_insert AFTER INSERT ON models_labels
BEGIN
    UPDATE models SET model_genesis_unchanged = 0 WHERE model_id = NEW.model_id;
END;
CREATE TRIGGER invalidate_model_genesis_label_delete AFTER DELETE ON models_labels
BEGIN
    UPDATE models SET model_genesis_unchanged = 0 WHERE model_id = OLD.model_id;
END;
CREATE TRIGGER invalidate_model_genesis_label_update AFTER UPDATE ON models_labels
WHEN OLD.model_id IS NOT NEW.model_id OR OLD.label_id IS NOT NEW.label_id
BEGIN
    UPDATE models SET model_genesis_unchanged = 0 WHERE model_id IN (OLD.model_id, NEW.model_id);
END;

-- Blob rows are shared; descriptor changes invalidate every referencing model.
CREATE TRIGGER invalidate_model_genesis_blob
AFTER UPDATE OF blob_sha256, blob_filetype, blob_size ON blobs
WHEN OLD.blob_sha256 IS NOT NEW.blob_sha256
    OR OLD.blob_filetype IS NOT NEW.blob_filetype
    OR OLD.blob_size IS NOT NEW.blob_size
BEGIN
    UPDATE models SET model_genesis_unchanged = 0 WHERE model_blob_id = NEW.blob_id;
END;
