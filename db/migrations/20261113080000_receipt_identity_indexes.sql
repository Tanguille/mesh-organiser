-- Only an envelope's authored entity is an identity claim; dependency references are not.
CREATE INDEX causal_receipt_global_claim ON causal_event_receipts (
    user_id,
    json_extract(envelope, '$.entity.kind'),
    json_extract(envelope, '$.entity.global_id'),
    json_extract(envelope, '$.entity.birth_id')
);

CREATE INDEX causal_receipt_birth_claim ON causal_event_receipts (
    user_id,
    json_extract(envelope, '$.entity.kind'),
    json_extract(envelope, '$.entity.birth_id'),
    json_extract(envelope, '$.entity.global_id')
) WHERE json_extract(envelope, '$.entity.birth_id') IS NOT NULL;

CREATE INDEX entity_births_owner_global ON entity_births(user_id, entity_kind, global_id, birth_id);
CREATE INDEX group_deletion_unknown_birth ON group_deletion_records(user_id, group_global_id) WHERE group_birth_id IS NULL;
