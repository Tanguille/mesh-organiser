CREATE TABLE sync_instance (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    instance_id TEXT NOT NULL UNIQUE
);

INSERT INTO sync_instance (singleton, instance_id) VALUES (1, lower(hex(randomblob(16))));

CREATE TABLE sync_accounts (
    user_id INTEGER PRIMARY KEY REFERENCES users(user_id) ON DELETE CASCADE,
    account_id TEXT NOT NULL UNIQUE
);

INSERT INTO sync_accounts (user_id, account_id)
SELECT user_id, lower(hex(randomblob(16))) FROM users;

CREATE TRIGGER create_sync_account AFTER INSERT ON users
BEGIN
    INSERT INTO sync_accounts (user_id, account_id)
    VALUES (NEW.user_id, lower(hex(randomblob(16))));
END;
