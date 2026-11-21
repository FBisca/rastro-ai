-- The validation contract a successful run recorded its output under (decisions/plugins.md §48):
-- the task template's declarations, saved so a replay checks the record against the rules that
-- applied to it, not the template currently selected. Only the latest good run keeps it; a run
-- recorded before contracts were saved has none and replays under the bundled legacy rules.
-- Forgetting a meeting deletes it with the rest of the retained replay data.
CREATE TABLE meeting_contract (
    run_id INTEGER PRIMARY KEY REFERENCES meeting_run (id),
    contract TEXT NOT NULL
) STRICT;