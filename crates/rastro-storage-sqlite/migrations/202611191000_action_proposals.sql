-- What an agent proposed to do in an outside system (actions.md): one row per proposal, bound to
-- the source it writes to and that source's scope, with the action's fields, the proposer's why
-- and the claims it cited. Insert-only: a proposal is never revised, and a repeat of the same
-- content is the same row. The id is the queue item's content address.
CREATE TABLE action_proposal (
    append_seq INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE CHECK (length(id) > 0),
    action TEXT NOT NULL CHECK (length(action) > 0),
    provider TEXT NOT NULL CHECK (length(provider) > 0),
    fields TEXT NOT NULL CHECK (json_valid(fields)),
    why TEXT NOT NULL CHECK (length(trim(why)) > 0),
    cites TEXT NOT NULL CHECK (json_valid(cites)),
    source TEXT NOT NULL CHECK (length(source) > 0),
    scope TEXT NOT NULL CHECK (length(scope) > 0),
    proposed_at TEXT NOT NULL CHECK (length(proposed_at) > 0)
) STRICT;
CREATE INDEX action_proposal_by_scope ON action_proposal (scope, append_seq);
CREATE TRIGGER action_proposal_no_update BEFORE UPDATE ON action_proposal
BEGIN SELECT RAISE(ABORT, 'action proposals are insert-only'); END;
CREATE TRIGGER action_proposal_no_delete BEFORE DELETE ON action_proposal
BEGIN SELECT RAISE(ABORT, 'action proposals are insert-only'); END;
CREATE TRIGGER action_proposal_no_replace BEFORE INSERT ON action_proposal
WHEN EXISTS (SELECT 1 FROM action_proposal WHERE append_seq = NEW.append_seq OR id = NEW.id)
BEGIN SELECT RAISE(ABORT, 'action proposals are insert-only'); END;
