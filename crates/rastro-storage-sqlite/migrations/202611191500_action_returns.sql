-- A pending Act whose re-check at grace end failed (actions.md): the rule it started under is gone
-- or no longer matches, a claim it cites was retracted, or an open observed issue holds its title.
-- Nothing was sent; the run reads returned and its proposal waits in the queue. At most one per run,
-- insert-only like a run's result: a run is never revised, and its status is derived.
CREATE TABLE action_return (
    append_seq INTEGER PRIMARY KEY,
    run_id TEXT NOT NULL UNIQUE REFERENCES action_run (id),
    reason TEXT NOT NULL CHECK (length(reason) > 0),
    returned_at TEXT NOT NULL CHECK (length(returned_at) > 0)
) STRICT;
CREATE TRIGGER action_return_no_update BEFORE UPDATE ON action_return
BEGIN SELECT RAISE(ABORT, 'action returns are insert-only'); END;
CREATE TRIGGER action_return_no_delete BEFORE DELETE ON action_return
BEGIN SELECT RAISE(ABORT, 'action returns are insert-only'); END;
CREATE TRIGGER action_return_no_replace BEFORE INSERT ON action_return
WHEN EXISTS (SELECT 1 FROM action_return WHERE append_seq = NEW.append_seq OR run_id = NEW.run_id)
BEGIN SELECT RAISE(ABORT, 'action returns are insert-only'); END;