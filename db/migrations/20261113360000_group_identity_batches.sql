-- Identity links authorize an additional observation, never a causal parent.
ALTER TABLE causal_observation_batches ADD COLUMN identity_link TEXT;
ALTER TABLE group_identity_observations ADD COLUMN batch_id TEXT;
CREATE UNIQUE INDEX group_identity_observation_batch ON group_identity_observations(user_id,batch_id) WHERE batch_id IS NOT NULL;
ALTER TABLE sync_group_identity_choices ADD COLUMN local_outbox_id TEXT;
