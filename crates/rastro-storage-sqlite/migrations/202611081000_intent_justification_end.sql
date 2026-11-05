-- An owner intent holds a claim over stretches: from its issue until it is undone, or until a
-- newer name replaces a Person's name; a name restored by undoing a newer one holds again from
-- that undo, a new row. A claim no stretch holds stays in the projection, superseded. `ended_at`
-- is NULL while the stretch holds. Part of the disposable projection: applying this migration
-- replays the stored intents, as `claims rebuild` and every intent write do.
ALTER TABLE intent_justification ADD COLUMN started_at TEXT;
ALTER TABLE intent_justification ADD COLUMN ended_at TEXT;
