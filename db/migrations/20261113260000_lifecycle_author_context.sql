-- Existing operations have no captured causal cut. Their context stays unknown;
-- reconstructing it from today's memberships would manufacture observations.
ALTER TABLE group_lifecycle_operations ADD COLUMN author_context TEXT
    CHECK(author_context IS NULL OR json_valid(author_context));
