ALTER TABLE resources ADD COLUMN resource_incarnation TEXT;
UPDATE resources SET resource_incarnation = lower(hex(randomblob(16)));
CREATE UNIQUE INDEX idx_resource_incarnation ON resources(resource_incarnation);
CREATE INDEX idx_resources_owner_global ON resources(resource_user_id, resource_unique_global_id);

CREATE TRIGGER initialize_resource_incarnation AFTER INSERT ON resources
BEGIN
    UPDATE resources SET resource_incarnation = lower(hex(randomblob(16))) WHERE resource_id = NEW.resource_id;
END;
CREATE TRIGGER immutable_resource_incarnation BEFORE UPDATE OF resource_incarnation ON resources
WHEN OLD.resource_incarnation IS NOT NULL OR NEW.resource_incarnation IS NULL
BEGIN
    SELECT RAISE(ABORT, 'resource incarnation is immutable');
END;
