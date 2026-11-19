CREATE TABLE sync_peer_bindings (
    user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
    local_instance_id TEXT NOT NULL,
    local_account_id TEXT NOT NULL,
    remote_instance_id TEXT NOT NULL,
    remote_account_id TEXT NOT NULL,
    PRIMARY KEY (user_id, local_instance_id, local_account_id, remote_instance_id, remote_account_id)
);
