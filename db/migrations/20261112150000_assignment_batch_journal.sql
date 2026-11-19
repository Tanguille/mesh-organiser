CREATE TABLE assignment_batch_journal (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (user_id, operation_id)
);
