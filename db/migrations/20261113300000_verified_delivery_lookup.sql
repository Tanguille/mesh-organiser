-- Reconnecting to the same authenticated hub at another origin must find exact
-- historical receipts without scanning unrelated delivery payloads.
CREATE INDEX sync_pull_receipt_verified_lookup ON sync_pull_receipts(kind,item_id,digest,issuer_connection_id,authority_writer_id,writer_id,stream_id,sequence);
