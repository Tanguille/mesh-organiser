-- Only future locally authored operations with a captured original delegation
-- enter automatic selection. Old history has no trustworthy enqueue/discard disposition.
CREATE TABLE sync_local_operation_selections (
 selection_id INTEGER PRIMARY KEY AUTOINCREMENT,
 user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 operation_id TEXT NOT NULL,
 writer_id TEXT NOT NULL,
 counter INTEGER NOT NULL CHECK(counter>0),
 authorization TEXT NOT NULL,
 outbox_id TEXT,
 UNIQUE(user_id,operation_id),
 FOREIGN KEY(user_id,operation_id) REFERENCES group_lifecycle_operations(user_id,operation_id) ON DELETE CASCADE,
 FOREIGN KEY(user_id,writer_id,counter) REFERENCES causal_events(user_id,writer_id,counter)
);
-- The original selected id survives ACK and explicit discard. Its absence from
-- the outbox is terminal; it never means this operation should be re-enqueued.
CREATE TRIGGER immutable_local_operation_selection BEFORE UPDATE ON sync_local_operation_selections
WHEN OLD.outbox_id IS NOT NULL OR NEW.outbox_id IS NULL
 OR OLD.selection_id IS NOT NEW.selection_id OR OLD.user_id IS NOT NEW.user_id
 OR OLD.operation_id IS NOT NEW.operation_id OR OLD.writer_id IS NOT NEW.writer_id
 OR OLD.counter IS NOT NEW.counter OR OLD.authorization IS NOT NEW.authorization
BEGIN SELECT RAISE(ABORT,'operation selection is immutable'); END;
