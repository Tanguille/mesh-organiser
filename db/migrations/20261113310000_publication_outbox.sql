-- Exact selected publications survive later domain edits. No historical events
-- are automatically enrolled, rebound or acknowledged by this migration.
-- Old certificates have no provable directory identity and remain fail-closed.
ALTER TABLE sync_transfer_intents ADD COLUMN directory_device TEXT;
ALTER TABLE sync_transfer_intents ADD COLUMN directory_inode TEXT;
CREATE TABLE sync_publication_outbox (
 outbox_id TEXT PRIMARY KEY,
 user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 local_instance_id TEXT NOT NULL,local_account_id TEXT NOT NULL,
 server_instance_id TEXT NOT NULL,server_account_id TEXT NOT NULL,
 connection_id TEXT NOT NULL,authority_writer_id TEXT NOT NULL,writer_id TEXT NOT NULL,
 writer_nonce TEXT NOT NULL,event_counter TEXT NOT NULL,
 kind TEXT NOT NULL,item_id TEXT NOT NULL,digest TEXT NOT NULL,item TEXT,
 ack_sequence TEXT,
 CHECK((ack_sequence IS NULL AND item IS NOT NULL) OR (ack_sequence IS NOT NULL AND item IS NULL)),
 UNIQUE(user_id,server_instance_id,server_account_id,kind,item_id)
);
CREATE TRIGGER immutable_publication_outbox BEFORE UPDATE ON sync_publication_outbox
WHEN NEW.outbox_id IS NOT OLD.outbox_id OR NEW.user_id IS NOT OLD.user_id
 OR NEW.local_instance_id IS NOT OLD.local_instance_id OR NEW.local_account_id IS NOT OLD.local_account_id
 OR NEW.server_instance_id IS NOT OLD.server_instance_id OR NEW.server_account_id IS NOT OLD.server_account_id
 OR NEW.connection_id IS NOT OLD.connection_id OR NEW.authority_writer_id IS NOT OLD.authority_writer_id
 OR NEW.writer_id IS NOT OLD.writer_id OR NEW.writer_nonce IS NOT OLD.writer_nonce OR NEW.event_counter IS NOT OLD.event_counter
 OR NEW.kind IS NOT OLD.kind OR NEW.item_id IS NOT OLD.item_id OR NEW.digest IS NOT OLD.digest OR NEW.item IS NOT NULL
 OR OLD.ack_sequence IS NOT NULL OR NEW.ack_sequence IS NULL
BEGIN SELECT RAISE(ABORT,'publication is immutable'); END;
CREATE INDEX pending_publication_owner ON sync_publication_outbox(user_id) WHERE ack_sequence IS NULL;
CREATE TABLE sync_publication_stages (
 outbox_id TEXT PRIMARY KEY REFERENCES sync_publication_outbox(outbox_id) ON DELETE CASCADE,
 stage_path TEXT NOT NULL UNIQUE REFERENCES sync_transfer_intents(stage_path) ON DELETE RESTRICT,
 token TEXT NOT NULL,files TEXT NOT NULL,logical_bytes INTEGER NOT NULL CHECK(logical_bytes>=0)
);
CREATE TRIGGER immutable_publication_stage BEFORE UPDATE ON sync_publication_stages
BEGIN SELECT RAISE(ABORT,'publication stage is immutable'); END;
