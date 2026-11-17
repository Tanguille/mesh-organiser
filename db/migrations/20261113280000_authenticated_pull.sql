CREATE TABLE sync_link_generations(user_id INTEGER PRIMARY KEY REFERENCES sync_accounts(user_id) ON DELETE CASCADE,generation INTEGER NOT NULL DEFAULT 0);
INSERT INTO sync_link_generations(user_id) SELECT user_id FROM sync_accounts;
CREATE TRIGGER sync_link_account AFTER INSERT ON sync_accounts BEGIN INSERT INTO sync_link_generations(user_id) VALUES(NEW.user_id); END;
CREATE TRIGGER sync_link_change AFTER UPDATE OF user_sync_url,user_sync_token ON users WHEN OLD.user_sync_url IS NOT NEW.user_sync_url OR OLD.user_sync_token IS NOT NEW.user_sync_token BEGIN UPDATE sync_link_generations SET generation=generation+1 WHERE user_id=NEW.user_id; END;
CREATE TRIGGER sync_link_authority_insert AFTER INSERT ON sync_transport_local_authorities BEGIN UPDATE sync_link_generations SET generation=generation+1 WHERE user_id=NEW.user_id; END;
CREATE TRIGGER sync_link_authority_delete AFTER DELETE ON sync_transport_local_authorities BEGIN UPDATE sync_link_generations SET generation=generation+1 WHERE user_id=OLD.user_id; END;
CREATE TABLE sync_pull_streams(stream_id TEXT PRIMARY KEY,user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,origin TEXT NOT NULL,server_instance_id TEXT NOT NULL,server_account_id TEXT NOT NULL,last_applied INTEGER NOT NULL DEFAULT 0,through INTEGER,UNIQUE(user_id,origin,server_instance_id,server_account_id));
CREATE TABLE sync_pull_receipts(stream_id TEXT NOT NULL REFERENCES sync_pull_streams(stream_id) ON DELETE CASCADE,sequence INTEGER NOT NULL,kind TEXT NOT NULL,item_id TEXT NOT NULL,digest TEXT NOT NULL,issuer_connection_id TEXT NOT NULL,authority_writer_id TEXT NOT NULL,writer_id TEXT NOT NULL,PRIMARY KEY(stream_id,sequence),UNIQUE(stream_id,kind,item_id));
CREATE TRIGGER sync_pull_receipt_immutable BEFORE UPDATE ON sync_pull_receipts BEGIN SELECT RAISE(ABORT,'immutable pull receipt'); END;
-- Only newly prepared, physically checked source exports gain this proof. Never
-- invent an export digest for older observation history during migration.
CREATE TABLE sync_observation_exports(user_id INTEGER NOT NULL,batch_id TEXT NOT NULL,digest TEXT NOT NULL,server_instance_id TEXT NOT NULL,server_account_id TEXT NOT NULL,connection_id TEXT NOT NULL,authority_writer_id TEXT NOT NULL,writer_id TEXT NOT NULL,PRIMARY KEY(user_id,batch_id),FOREIGN KEY(user_id,batch_id) REFERENCES causal_observation_batches(user_id,batch_id) ON DELETE CASCADE);
CREATE TRIGGER sync_observation_export_immutable BEFORE UPDATE ON sync_observation_exports BEGIN SELECT RAISE(ABORT,'immutable source export'); END;
