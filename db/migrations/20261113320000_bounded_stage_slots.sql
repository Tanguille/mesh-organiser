-- Names are reserved before mkdir. An interrupted, uncertified reservation is
-- retained against capacity, never adopted by guessing filesystem ownership.
CREATE TABLE sync_stage_slots (
 stage_path TEXT PRIMARY KEY,
 root_path TEXT NOT NULL,
 root_device TEXT NOT NULL,
 root_inode TEXT NOT NULL,
 database_path TEXT NOT NULL,
 database_device TEXT NOT NULL,
 database_inode TEXT NOT NULL,
 directory_device TEXT,
 directory_inode TEXT,
 lease_device TEXT,
 lease_inode TEXT,
 CHECK ((directory_device IS NULL AND directory_inode IS NULL AND lease_device IS NULL AND lease_inode IS NULL)
     OR (directory_device IS NOT NULL AND directory_inode IS NOT NULL AND lease_device IS NOT NULL AND lease_inode IS NOT NULL))
);
CREATE TRIGGER immutable_stage_slot BEFORE UPDATE ON sync_stage_slots
WHEN OLD.directory_device IS NOT NULL
 OR NEW.directory_device IS NULL
 OR NEW.stage_path IS NOT OLD.stage_path
 OR NEW.root_path IS NOT OLD.root_path
 OR NEW.root_device IS NOT OLD.root_device
 OR NEW.root_inode IS NOT OLD.root_inode
 OR NEW.database_path IS NOT OLD.database_path
 OR NEW.database_device IS NOT OLD.database_device
 OR NEW.database_inode IS NOT OLD.database_inode
BEGIN SELECT RAISE(ABORT,'stage slot certificate is immutable'); END;
CREATE TRIGGER retain_stage_slot BEFORE DELETE ON sync_stage_slots
BEGIN SELECT RAISE(ABORT,'stage slot reservation must be retained'); END;
