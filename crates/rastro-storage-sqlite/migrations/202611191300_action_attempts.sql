-- An execution of an action run that failed (actions.md): the error it ended with, one row per
-- failed attempt, numbered from 1 in the order they were made. Insert-only, like the run and its
-- result: a run reads `failed` when it has attempts and no result, and the attempts it has made are
-- the rows here, so `tick` stops retrying at 3 and nothing is updated in place.
CREATE TABLE action_attempt (
    append_seq INTEGER PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES action_run (id),
    attempt INTEGER NOT NULL CHECK (attempt >= 1),
    error TEXT NOT NULL CHECK (length(error) > 0),
    failed_at TEXT NOT NULL CHECK (length(failed_at) > 0),
    UNIQUE (run_id, attempt)
) STRICT;
CREATE TRIGGER action_attempt_no_update BEFORE UPDATE ON action_attempt
BEGIN SELECT RAISE(ABORT, 'action attempts are insert-only'); END;
CREATE TRIGGER action_attempt_no_delete BEFORE DELETE ON action_attempt
BEGIN SELECT RAISE(ABORT, 'action attempts are insert-only'); END;
CREATE TRIGGER action_attempt_no_replace BEFORE INSERT ON action_attempt
WHEN EXISTS (SELECT 1 FROM action_attempt WHERE append_seq = NEW.append_seq)
BEGIN SELECT RAISE(ABORT, 'action attempts are insert-only'); END;
