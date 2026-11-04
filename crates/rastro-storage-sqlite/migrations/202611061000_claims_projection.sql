-- Extractor runs share the run log with connector runs but have no window and no coverage.
ALTER TABLE runs ADD COLUMN role TEXT NOT NULL DEFAULT 'connector'
    CHECK (role IN ('connector', 'extractor'));
-- The extractor a source's connector binds it to, as last seen by a sync.
ALTER TABLE sources ADD COLUMN extractor TEXT;
-- The claims projection: derived from evidence, disposable, rebuilt by replay.
CREATE TABLE entity (
    id TEXT PRIMARY KEY, type TEXT NOT NULL, identifier TEXT NOT NULL UNIQUE
);
CREATE TABLE alias (
    entity_id TEXT NOT NULL REFERENCES entity(id), alias TEXT NOT NULL, kind TEXT NOT NULL,
    PRIMARY KEY (entity_id, alias, kind)
) WITHOUT ROWID;
CREATE TABLE claim (
    id TEXT PRIMARY KEY, subject TEXT NOT NULL REFERENCES entity(id), predicate TEXT NOT NULL,
    object_entity TEXT REFERENCES entity(id), object_value TEXT, object_datatype TEXT,
    status TEXT NOT NULL CHECK (status IN ('observed', 'superseded')), revision TEXT NOT NULL,
    CHECK ((object_entity IS NULL) = (object_value IS NOT NULL AND object_datatype IS NOT NULL))
);
CREATE TABLE claim_support (
    claim_id TEXT NOT NULL REFERENCES claim(id), evidence_id INTEGER NOT NULL REFERENCES evidence(id),
    field TEXT NOT NULL, start INTEGER NOT NULL, end INTEGER NOT NULL CHECK (end > start),
    extraction_rule TEXT NOT NULL, extractor TEXT NOT NULL, extractor_version TEXT NOT NULL,
    run_id INTEGER NOT NULL REFERENCES runs(id),
    current INTEGER NOT NULL DEFAULT 1 CHECK (current IN (0, 1)),
    PRIMARY KEY (claim_id, evidence_id, field, start, end, extraction_rule)
) WITHOUT ROWID;
CREATE INDEX claim_by_subject ON claim (subject);
CREATE INDEX claim_by_object ON claim (object_entity);
CREATE TABLE extraction_checkpoint (
    extractor TEXT NOT NULL, extractor_version TEXT NOT NULL, source TEXT NOT NULL,
    scope TEXT NOT NULL, last_captured_at TEXT NOT NULL, last_evidence_id INTEGER NOT NULL,
    PRIMARY KEY (extractor, extractor_version, source, scope)
) WITHOUT ROWID;
-- Replay order within a pair, and the floor that keeps capture times from going backwards.
CREATE INDEX evidence_by_pair_capture ON evidence (source, scope, captured_at, id);
