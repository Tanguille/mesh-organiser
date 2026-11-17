CREATE TABLE local_creation_records (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    entity_kind TEXT NOT NULL CHECK (entity_kind IN ('group', 'model')),
    birth_id TEXT NOT NULL,
    writer_id TEXT NOT NULL,
    counter INTEGER NOT NULL CHECK (counter > 0),
    envelope TEXT NOT NULL,
    PRIMARY KEY (user_id, entity_kind, birth_id),
    UNIQUE (writer_id, counter),
    FOREIGN KEY (user_id, entity_kind, birth_id) REFERENCES entity_births(user_id, entity_kind, birth_id)
);
CREATE TRIGGER immutable_local_creation_record BEFORE UPDATE ON local_creation_records
BEGIN
    SELECT RAISE(ABORT, 'local creation record is immutable');
END;
