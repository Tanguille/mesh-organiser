-- Experimental hub authority. Caller-provided device identities are pins only.
CREATE TABLE sync_transport_writers (
    writer_id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
    account_id TEXT NOT NULL,
    connection_id TEXT NOT NULL UNIQUE,
    secret_hash TEXT NOT NULL,
    client_instance_id TEXT NOT NULL,
    client_account_id TEXT NOT NULL
);
CREATE TABLE sync_transport_local_writers (
    writer_id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
    local_instance_id TEXT NOT NULL,
    local_account_id TEXT NOT NULL,
    server_instance_id TEXT NOT NULL,
    server_account_id TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    secret_hash TEXT NOT NULL
);
CREATE TABLE sync_transport_log (
    user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK(sequence>0),
    kind TEXT NOT NULL CHECK(kind IN ('group_genesis','group_operation')),
    item_id TEXT NOT NULL,
    digest TEXT NOT NULL,
    author_writer_id TEXT NOT NULL REFERENCES sync_transport_writers(writer_id),
    PRIMARY KEY(user_id,sequence),
    UNIQUE(user_id,kind,item_id)
);
CREATE TRIGGER immutable_sync_transport_log BEFORE UPDATE ON sync_transport_log
BEGIN SELECT RAISE(ABORT,'transport receipt is immutable'); END;
