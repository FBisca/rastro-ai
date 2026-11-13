-- Meetings imported from speaker-labelled transcripts, their extraction runs, and the cited
-- commitment hypotheses a successful run proposed. The transcript itself is not stored.
CREATE TABLE meeting (
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL,
    captured_at TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;
CREATE TABLE meeting_run (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    meeting_id TEXT NOT NULL REFERENCES meeting (id),
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('succeeded', 'failed')),
    cause TEXT,
    reason TEXT,
    extractor TEXT,
    owner_speaker TEXT,
    turns_total INTEGER NOT NULL,
    turns_read INTEGER NOT NULL,
    excluded INTEGER NOT NULL,
    CHECK ((state = 'failed') = (cause IS NOT NULL))
) STRICT;
CREATE INDEX meeting_run_by_meeting ON meeting_run (meeting_id, id);
CREATE TABLE meeting_hypothesis (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES meeting_run (id),
    ordinal INTEGER NOT NULL,
    responsible_label TEXT,
    responsible_entity TEXT,
    responsible_identifier TEXT,
    recipient_label TEXT,
    recipient_entity TEXT,
    recipient_identifier TEXT,
    promise TEXT NOT NULL,
    stated_date TEXT,
    turn INTEGER NOT NULL,
    speaker TEXT NOT NULL,
    quote TEXT NOT NULL,
    quote_start INTEGER NOT NULL,
    quote_end INTEGER NOT NULL,
    UNIQUE (run_id, ordinal)
) STRICT;
