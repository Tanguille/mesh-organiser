-- Credentials use the existing local database secret-at-rest boundary. They are
-- deliberately separate from public proof, delivery and snapshot tables.
CREATE TABLE sync_transport_local_authorities (
 user_id INTEGER PRIMARY KEY REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 local_instance_id TEXT NOT NULL,local_account_id TEXT NOT NULL,
 server_instance_id TEXT NOT NULL,server_account_id TEXT NOT NULL,
 connection_id TEXT NOT NULL,authority_writer_id TEXT NOT NULL,secret TEXT NOT NULL
);
CREATE TABLE causal_account_writers (
 user_id INTEGER PRIMARY KEY REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 writer_id TEXT NOT NULL UNIQUE,counter INTEGER NOT NULL CHECK(typeof(counter)='integer' AND counter>=0)
);
CREATE TABLE local_writer_delegations (
 writer_id TEXT PRIMARY KEY,user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 account_id TEXT NOT NULL,connection_id TEXT NOT NULL,writer_nonce TEXT NOT NULL,
 server_instance_id TEXT NOT NULL,server_account_id TEXT NOT NULL,
 UNIQUE(user_id,connection_id,writer_nonce)
);
CREATE TABLE sync_transport_delegations (
 writer_id TEXT PRIMARY KEY,user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 account_id TEXT NOT NULL,authority_writer_id TEXT NOT NULL REFERENCES sync_transport_writers(writer_id),
 writer_nonce TEXT NOT NULL,UNIQUE(authority_writer_id,writer_nonce)
);
CREATE TRIGGER immutable_local_writer_delegation BEFORE UPDATE ON local_writer_delegations
BEGIN SELECT RAISE(ABORT,'writer delegation is immutable'); END;
CREATE TRIGGER immutable_transport_delegation BEFORE UPDATE ON sync_transport_delegations
BEGIN SELECT RAISE(ABORT,'writer delegation is immutable'); END;
-- A relay records the authenticated authority separately from the original dot's writer.
CREATE TABLE sync_transport_log_new (
 user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 sequence INTEGER NOT NULL CHECK(sequence>0),kind TEXT NOT NULL CHECK(kind IN ('group_genesis','group_operation')),
 item_id TEXT NOT NULL,digest TEXT NOT NULL,
 authority_writer_id TEXT NOT NULL REFERENCES sync_transport_writers(writer_id),
 writer_id TEXT NOT NULL,
 PRIMARY KEY(user_id,sequence),UNIQUE(user_id,kind,item_id)
);
INSERT INTO sync_transport_log_new SELECT user_id,sequence,kind,item_id,digest,author_writer_id,author_writer_id FROM sync_transport_log;
DROP TABLE sync_transport_log;
ALTER TABLE sync_transport_log_new RENAME TO sync_transport_log;
CREATE TRIGGER immutable_sync_transport_log BEFORE UPDATE ON sync_transport_log
BEGIN SELECT RAISE(ABORT,'transport receipt is immutable'); END;
CREATE TRIGGER clear_offline_authority_after_unlink AFTER UPDATE OF user_sync_token,user_sync_url ON users
WHEN OLD.user_sync_token IS NOT NEW.user_sync_token OR OLD.user_sync_url IS NOT NEW.user_sync_url
BEGIN
 DELETE FROM sync_transport_local_authorities WHERE user_id=OLD.user_id;
 DELETE FROM causal_account_writers WHERE user_id=OLD.user_id;
END;
