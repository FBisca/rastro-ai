CREATE TABLE sources (
    name TEXT PRIMARY KEY, connector TEXT NOT NULL, selector TEXT NOT NULL,
    scope TEXT NOT NULL, freshness_rule TEXT NOT NULL, window_start TEXT NOT NULL,
    keep_original INTEGER NOT NULL CHECK (keep_original IN (0, 1))
);
CREATE TABLE runs (
    id INTEGER PRIMARY KEY, source TEXT NOT NULL REFERENCES sources(name), scope TEXT NOT NULL,
    plugin_id TEXT NOT NULL, plugin_version TEXT NOT NULL, started_at TEXT NOT NULL,
    finished_at TEXT, status TEXT NOT NULL, reason TEXT,
    submitted INTEGER NOT NULL DEFAULT 0, accepted INTEGER NOT NULL DEFAULT 0,
    rejected INTEGER NOT NULL DEFAULT 0, unchanged INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE coverage (
    id INTEGER PRIMARY KEY, run_id INTEGER NOT NULL REFERENCES runs(id),
    source TEXT NOT NULL REFERENCES sources(name), scope TEXT NOT NULL,
    window_start TEXT NOT NULL, window_end TEXT NOT NULL, selection TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('found', 'empty', 'couldn''t look')),
    reason TEXT, retry_at TEXT, checked_at TEXT NOT NULL
);
CREATE TABLE evidence (
    id INTEGER PRIMARY KEY, source TEXT NOT NULL REFERENCES sources(name), scope TEXT NOT NULL,
    location TEXT NOT NULL, field TEXT NOT NULL, start INTEGER NOT NULL CHECK (start >= 0),
    end INTEGER NOT NULL CHECK (end >= start), quote TEXT NOT NULL,
    captured_at TEXT NOT NULL, source_updated_at TEXT,
    trust_class TEXT NOT NULL CHECK (trust_class IN ('structured', 'unstructured', 'untrusted content')),
    original TEXT, CHECK (length(CAST(quote AS BLOB)) = end - start)
);
CREATE TRIGGER evidence_no_update BEFORE UPDATE ON evidence
BEGIN SELECT RAISE(ABORT, 'evidence is insert-only'); END;
CREATE TRIGGER evidence_no_delete BEFORE DELETE ON evidence
BEGIN SELECT RAISE(ABORT, 'evidence is insert-only'); END;
