-- An owner intent's contribution to a claim ends when the intent is undone, or when a newer name
-- replaces a Person's name; the claim then stays in the projection, superseded, its interval
-- closed at that time. NULL while the contribution holds. Part of the disposable projection:
-- `claims rebuild` (or the next intent write) fills it from the intent table.
ALTER TABLE intent_justification ADD COLUMN ended_at TEXT;
