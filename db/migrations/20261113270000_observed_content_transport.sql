-- Proof source identities remain claims. Receipt reuse uses the authenticated
-- issuer namespace, without changing any original observation proof or event.
ALTER TABLE causal_observation_batches ADD COLUMN verified_source_instance TEXT;
ALTER TABLE causal_observation_batches ADD COLUMN verified_source_account TEXT;
CREATE TABLE sync_observation_contents (
 user_id INTEGER NOT NULL,
 batch_id TEXT NOT NULL,
 hash TEXT NOT NULL,
 filetype TEXT NOT NULL,
 size INTEGER NOT NULL CHECK(size>=0),
 sha256 TEXT NOT NULL,
 blob_id INTEGER REFERENCES blobs(blob_id) ON DELETE SET NULL,
 PRIMARY KEY(user_id,batch_id,hash),
 FOREIGN KEY(user_id,batch_id) REFERENCES causal_observation_batches(user_id,batch_id) ON DELETE CASCADE
);
CREATE TRIGGER immutable_sync_observation_content BEFORE UPDATE ON sync_observation_contents
WHEN NEW.user_id IS NOT OLD.user_id OR NEW.batch_id IS NOT OLD.batch_id OR NEW.hash IS NOT OLD.hash OR NEW.filetype IS NOT OLD.filetype OR NEW.size IS NOT OLD.size OR NEW.sha256 IS NOT OLD.sha256 OR NEW.blob_id IS NOT NULL
BEGIN SELECT RAISE(ABORT,'verified content reference is immutable'); END;
-- Only generated application-owned paths enter this ledger. External files and
-- shared originals are never deleted as a failed transfer's cleanup.
CREATE TABLE sync_owned_blob_files (
 path TEXT PRIMARY KEY,
 blob_id INTEGER UNIQUE REFERENCES blobs(blob_id) ON DELETE SET NULL,
 sha256 TEXT NOT NULL,
 device TEXT NOT NULL,
 inode TEXT NOT NULL
);
CREATE TABLE sync_transport_log_new (
 user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 sequence INTEGER NOT NULL CHECK(sequence>0),
 kind TEXT NOT NULL CHECK(kind IN ('group_genesis','group_operation','observed_group')),
 item_id TEXT NOT NULL,digest TEXT NOT NULL,
 authority_writer_id TEXT NOT NULL REFERENCES sync_transport_writers(writer_id),
 writer_id TEXT NOT NULL,
 PRIMARY KEY(user_id,sequence),UNIQUE(user_id,kind,item_id)
);
INSERT INTO sync_transport_log_new SELECT * FROM sync_transport_log;
DROP TABLE sync_transport_log;
ALTER TABLE sync_transport_log_new RENAME TO sync_transport_log;
CREATE TRIGGER immutable_sync_transport_log BEFORE UPDATE ON sync_transport_log
BEGIN SELECT RAISE(ABORT,'transport receipt is immutable'); END;
-- A stage is certified before content is written. Final-link inode evidence is
-- persisted before publication, independently of the later domain transaction.
CREATE TABLE sync_transfer_intents (
 stage_path TEXT PRIMARY KEY,
 token TEXT NOT NULL,
 manifest TEXT NOT NULL DEFAULT '[]'
);
