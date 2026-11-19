-- Retain the original issuing authority independently of active credentials. A
-- lost historical registration cannot be inferred from a caller's claimed id.
ALTER TABLE local_writer_delegations ADD COLUMN authority_writer_id TEXT;
DROP TRIGGER immutable_local_writer_delegation;
UPDATE local_writer_delegations AS d SET authority_writer_id=(
 SELECT a.authority_writer_id FROM sync_transport_local_authorities a
 WHERE a.user_id=d.user_id AND a.local_account_id=d.account_id
 AND a.connection_id=d.connection_id AND a.server_instance_id=d.server_instance_id
 AND a.server_account_id=d.server_account_id
);
CREATE TRIGGER immutable_local_writer_delegation BEFORE UPDATE ON local_writer_delegations
BEGIN SELECT RAISE(ABORT,'writer delegation is immutable'); END;
