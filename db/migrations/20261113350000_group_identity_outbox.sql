-- The decision keeps its publication id even after publication or explicit cancellation.
ALTER TABLE group_identity_separations ADD COLUMN local_outbox_id TEXT;
