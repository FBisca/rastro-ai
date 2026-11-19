-- foreign_keys = off
-- A rule the owner wrote can now start a run at Act (actions.md): the run holds no accept intent,
-- the level it started at, a copy of the rule that authorized it and the time its grace ends.
-- SQLite can't drop a column's NOT NULL in place, so the table is rebuilt the way its documentation
-- says: the open runs this migration with foreign keys off, then checks them before it commits.
-- Every run is kept: runs recorded before this migration were all owner accepts, at level propose.
CREATE TABLE action_run_rebuilt (
    append_seq INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE CHECK (length(id) > 0),
    proposal TEXT NOT NULL UNIQUE REFERENCES action_proposal (id),
    accept_intent TEXT UNIQUE REFERENCES intent (id),
    action TEXT NOT NULL CHECK (length(action) > 0),
    provider TEXT NOT NULL CHECK (length(provider) > 0),
    fields TEXT NOT NULL CHECK (json_valid(fields)),
    source TEXT NOT NULL CHECK (length(source) > 0),
    scope TEXT NOT NULL CHECK (length(scope) > 0),
    issue_id TEXT NOT NULL UNIQUE CHECK (length(issue_id) > 0),
    accepted_at TEXT NOT NULL CHECK (length(accepted_at) > 0),
    level TEXT NOT NULL DEFAULT 'propose' CHECK (level IN ('propose', 'act')),
    rule TEXT CHECK (rule IS NULL OR json_valid(rule)),
    due_at TEXT CHECK (due_at IS NULL OR length(due_at) > 0),
    CHECK (
        (level = 'propose' AND accept_intent IS NOT NULL AND rule IS NULL AND due_at IS NULL)
        OR (level = 'act' AND accept_intent IS NULL AND rule IS NOT NULL AND due_at IS NOT NULL)
    )
) STRICT;
INSERT INTO action_run_rebuilt (append_seq, id, proposal, accept_intent, action, provider, fields,
    source, scope, issue_id, accepted_at, level)
SELECT append_seq, id, proposal, accept_intent, action, provider, fields, source, scope, issue_id,
    accepted_at, 'propose' FROM action_run;
DROP TABLE action_run;
ALTER TABLE action_run_rebuilt RENAME TO action_run;
CREATE INDEX action_run_by_scope ON action_run (scope, append_seq);
CREATE TRIGGER action_run_no_update BEFORE UPDATE ON action_run
BEGIN SELECT RAISE(ABORT, 'action runs are insert-only'); END;
CREATE TRIGGER action_run_no_delete BEFORE DELETE ON action_run
BEGIN SELECT RAISE(ABORT, 'action runs are insert-only'); END;
CREATE TRIGGER action_run_no_replace BEFORE INSERT ON action_run
WHEN EXISTS (SELECT 1 FROM action_run WHERE append_seq = NEW.append_seq OR id = NEW.id)
BEGIN SELECT RAISE(ABORT, 'action runs are insert-only'); END;
