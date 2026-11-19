-- A received conflict offer is authorized by an applied transport receipt, not a fabricated identity decision.
CREATE TABLE received_group_name_choices (
 choice_id TEXT PRIMARY KEY,
 user_id INTEGER NOT NULL REFERENCES sync_accounts(user_id) ON DELETE CASCADE,
 authorization TEXT NOT NULL,
 birth_id TEXT NOT NULL,
 token TEXT NOT NULL,
 evidence TEXT NOT NULL,
 selected_name TEXT,
 writer_id TEXT,
 counter INTEGER,
 outbox_id TEXT,
 CHECK ((selected_name IS NULL)=(writer_id IS NULL)),
 CHECK ((selected_name IS NULL)=(counter IS NULL)),
 CHECK ((selected_name IS NULL)=(outbox_id IS NULL)),
 FOREIGN KEY(user_id,writer_id,counter) REFERENCES causal_events(user_id,writer_id,counter)
);
CREATE TRIGGER immutable_received_name_choice BEFORE UPDATE ON received_group_name_choices
WHEN OLD.selected_name IS NOT NULL
 OR OLD.choice_id IS NOT NEW.choice_id OR OLD.user_id IS NOT NEW.user_id
 OR OLD.authorization IS NOT NEW.authorization OR OLD.birth_id IS NOT NEW.birth_id
 OR OLD.token IS NOT NEW.token OR OLD.evidence IS NOT NEW.evidence
BEGIN SELECT RAISE(ABORT,'received name offer is immutable'); END;
