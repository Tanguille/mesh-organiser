-- Every new unknown-identity departure is distinct. Old global-only markers
-- remain uncertifiable: no migration can recover their missing incarnations.
CREATE TABLE unknown_model_identity_evidence (
 evidence_id INTEGER PRIMARY KEY AUTOINCREMENT,
 user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
 model_global_id TEXT NOT NULL,
 incarnation TEXT,
 reason TEXT NOT NULL CHECK(reason IN ('legacy','rekey','delete'))
);
CREATE INDEX unknown_model_identity_evidence_global ON unknown_model_identity_evidence(user_id,model_global_id);
INSERT INTO unknown_model_identity_evidence(user_id,model_global_id,reason)
 SELECT user_id,model_global_id,'legacy' FROM unknown_model_identity_tombstones;
CREATE TRIGGER immutable_unknown_model_evidence BEFORE UPDATE ON unknown_model_identity_evidence
BEGIN SELECT RAISE(ABORT,'unknown model evidence is immutable'); END;
CREATE TRIGGER journal_unknown_model_delete BEFORE DELETE ON models
WHEN OLD.model_birth_id IS NULL AND EXISTS(SELECT 1 FROM users WHERE user_id=OLD.model_user_id)
BEGIN INSERT INTO unknown_model_identity_evidence(user_id,model_global_id,incarnation,reason)
 VALUES(OLD.model_user_id,OLD.model_unique_global_id,OLD.model_incarnation,'delete'); END;
CREATE TRIGGER journal_unknown_model_rekey BEFORE UPDATE OF model_unique_global_id ON models
WHEN OLD.model_birth_id IS NULL AND OLD.model_unique_global_id IS NOT NEW.model_unique_global_id
BEGIN INSERT INTO unknown_model_identity_evidence(user_id,model_global_id,incarnation,reason)
 VALUES(OLD.model_user_id,OLD.model_unique_global_id,OLD.model_incarnation,'rekey'); END;

CREATE TABLE sync_model_identity_choices (
 decision_id TEXT PRIMARY KEY REFERENCES sync_group_identity_choices(decision_id) ON DELETE CASCADE,
 user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
 local_model_id INTEGER NOT NULL,
 group_choice TEXT CHECK(group_choice IN ('same','separate')),
 remote_group_id INTEGER,
 choice TEXT CHECK(choice IN ('same','separate')),
 remote_model_id INTEGER,
 remote_global_id TEXT NOT NULL,
 remote_birth_id TEXT NOT NULL,
 local_global_id TEXT,
 local_birth_id TEXT
);
CREATE TRIGGER immutable_model_identity_choice BEFORE UPDATE ON sync_model_identity_choices
WHEN OLD.choice IS NOT NULL
 OR OLD.decision_id IS NOT NEW.decision_id OR OLD.user_id IS NOT NEW.user_id
 OR OLD.local_model_id IS NOT NEW.local_model_id OR OLD.remote_global_id IS NOT NEW.remote_global_id
 OR OLD.remote_birth_id IS NOT NEW.remote_birth_id
 OR NOT (
   (OLD.group_choice IS NULL AND NEW.group_choice IS NOT NULL AND NEW.choice IS NULL
    AND NEW.remote_model_id IS NULL AND NEW.remote_group_id IS NULL AND NEW.local_global_id IS NULL AND NEW.local_birth_id IS NULL)
   OR (OLD.group_choice IS NOT NULL AND OLD.group_choice IS NEW.group_choice AND NEW.choice IS NOT NULL
       AND NEW.remote_model_id IS NOT NULL AND NEW.remote_group_id IS NOT NULL AND NEW.local_global_id IS NOT NULL AND NEW.local_birth_id IS NOT NULL)
 )
BEGIN SELECT RAISE(ABORT,'model identity choice is immutable'); END;

CREATE TABLE model_identity_separation_evidence (
 decision_id TEXT PRIMARY KEY REFERENCES sync_model_identity_choices(decision_id),
 user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
 evidence_id INTEGER NOT NULL UNIQUE REFERENCES unknown_model_identity_evidence(evidence_id),
 remote_global_id TEXT NOT NULL,
 remote_birth_id TEXT NOT NULL,
 local_global_id TEXT NOT NULL
);
CREATE TRIGGER immutable_model_separation_evidence BEFORE UPDATE ON model_identity_separation_evidence
BEGIN SELECT RAISE(ABORT,'model separation evidence is immutable'); END;

CREATE TABLE model_identity_observations (
 user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
 batch_id TEXT NOT NULL,
 birth_id TEXT NOT NULL,
 writer_id TEXT NOT NULL,
 counter INTEGER NOT NULL CHECK(counter>0),
 root TEXT NOT NULL,
 materialization TEXT NOT NULL,
 proof TEXT NOT NULL,
 PRIMARY KEY(user_id,writer_id,counter),
 UNIQUE(user_id,batch_id),
 FOREIGN KEY(user_id,batch_id) REFERENCES causal_observation_batches(user_id,batch_id),
 FOREIGN KEY(user_id,writer_id,counter) REFERENCES causal_events(user_id,writer_id,counter)
);
CREATE TRIGGER immutable_model_identity_observation BEFORE UPDATE ON model_identity_observations
BEGIN SELECT RAISE(ABORT,'model identity observation is immutable'); END;
