-- What the owner's accept of an action proposal started (actions.md): one row per run, written in
-- the same transaction as the accept intent, holding the random issue id the outside system is
-- asked to create under. Insert-only: a run is never revised, and a run's outcome is its own
-- row below, so a run's status is derived and nothing is updated in place. One run per proposal.
CREATE TABLE action_run (
    append_seq INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE CHECK (length(id) > 0),
    proposal TEXT NOT NULL UNIQUE REFERENCES action_proposal (id),
    accept_intent TEXT NOT NULL UNIQUE REFERENCES intent (id),
    action TEXT NOT NULL CHECK (length(action) > 0),
    provider TEXT NOT NULL CHECK (length(provider) > 0),
    fields TEXT NOT NULL CHECK (json_valid(fields)),
    source TEXT NOT NULL CHECK (length(source) > 0),
    scope TEXT NOT NULL CHECK (length(scope) > 0),
    issue_id TEXT NOT NULL UNIQUE CHECK (length(issue_id) > 0),
    accepted_at TEXT NOT NULL CHECK (length(accepted_at) > 0)
) STRICT;
CREATE INDEX action_run_by_scope ON action_run (scope, append_seq);
CREATE TRIGGER action_run_no_update BEFORE UPDATE ON action_run
BEGIN SELECT RAISE(ABORT, 'action runs are insert-only'); END;
CREATE TRIGGER action_run_no_delete BEFORE DELETE ON action_run
BEGIN SELECT RAISE(ABORT, 'action runs are insert-only'); END;
CREATE TRIGGER action_run_no_replace BEFORE INSERT ON action_run
WHEN EXISTS (SELECT 1 FROM action_run WHERE append_seq = NEW.append_seq OR id = NEW.id)
BEGIN SELECT RAISE(ABORT, 'action runs are insert-only'); END;

-- A run that created its issue: the identifier and URL the outside system gave it. At most one per
-- run, so a run reads done once however often it is executed.
CREATE TABLE action_result (
    append_seq INTEGER PRIMARY KEY,
    run_id TEXT NOT NULL UNIQUE REFERENCES action_run (id),
    identifier TEXT NOT NULL CHECK (length(identifier) > 0),
    url TEXT NOT NULL CHECK (length(url) > 0),
    finished_at TEXT NOT NULL CHECK (length(finished_at) > 0)
) STRICT;
CREATE TRIGGER action_result_no_update BEFORE UPDATE ON action_result
BEGIN SELECT RAISE(ABORT, 'action results are insert-only'); END;
CREATE TRIGGER action_result_no_delete BEFORE DELETE ON action_result
BEGIN SELECT RAISE(ABORT, 'action results are insert-only'); END;
CREATE TRIGGER action_result_no_replace BEFORE INSERT ON action_result
WHEN EXISTS (SELECT 1 FROM action_result WHERE append_seq = NEW.append_seq OR run_id = NEW.run_id)
BEGIN SELECT RAISE(ABORT, 'action results are insert-only'); END;
