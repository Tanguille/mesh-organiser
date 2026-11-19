ALTER TABLE membership_causal_events ADD COLUMN from_group_global_id TEXT;
ALTER TABLE membership_causal_events ADD COLUMN to_group_global_id TEXT;
ALTER TABLE group_causal_events ADD COLUMN model_global_id TEXT;
ALTER TABLE group_causal_events ADD COLUMN model_incarnation TEXT;
ALTER TABLE group_causal_events ADD COLUMN membership_replica_id TEXT;
ALTER TABLE group_causal_events ADD COLUMN membership_counter INTEGER;

CREATE INDEX idx_models_owner_global ON models(model_user_id, model_unique_global_id);
