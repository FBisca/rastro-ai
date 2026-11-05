-- The projection is disposable. Evidence and extractor run records are retained.
DROP TABLE claim_support;
DROP TABLE alias;
DROP TABLE claim;
DROP TABLE entity;
DROP TABLE extraction_checkpoint;

CREATE TABLE entity (id TEXT PRIMARY KEY, identifier TEXT NOT NULL UNIQUE);
CREATE TABLE claim (
    id TEXT PRIMARY KEY, subject TEXT NOT NULL REFERENCES entity(id), predicate TEXT NOT NULL,
    object_entity TEXT REFERENCES entity(id), object_value TEXT, object_datatype TEXT,
    scalar INTEGER NOT NULL CHECK (scalar IN (0, 1)),
    status TEXT NOT NULL CHECK (status IN ('observed', 'superseded')), revision TEXT NOT NULL,
    known_since TEXT NOT NULL DEFAULT '',
    CHECK ((object_entity IS NULL) = (object_value IS NOT NULL AND object_datatype IS NOT NULL))
);
CREATE INDEX claim_by_subject ON claim (subject, known_since DESC, id);
CREATE INDEX claim_by_object ON claim (object_entity, known_since DESC, id);
CREATE INDEX claim_alias ON claim (predicate, object_value) WHERE predicate = 'rastro:alias';
CREATE TABLE justification (
    id TEXT PRIMARY KEY, claim_id TEXT NOT NULL REFERENCES claim(id),
    kind TEXT NOT NULL CHECK (kind IN ('provenance', 'inference', 'confirmation')),
    extractor TEXT NOT NULL, extractor_version TEXT NOT NULL, vocabulary_version TEXT NOT NULL,
    derivation_key TEXT NOT NULL, extraction_rule TEXT NOT NULL,
    run_id INTEGER NOT NULL REFERENCES runs(id), current INTEGER NOT NULL DEFAULT 0 CHECK (current IN (0, 1))
);
CREATE INDEX justification_by_claim ON justification (claim_id);
CREATE TABLE justification_span (
    justification_id TEXT NOT NULL REFERENCES justification(id),
    evidence_id INTEGER NOT NULL REFERENCES evidence(id), portable_key TEXT NOT NULL,
    field TEXT NOT NULL, start INTEGER NOT NULL, end INTEGER NOT NULL CHECK (end > start),
    PRIMARY KEY (justification_id, portable_key, field, start, end, evidence_id)
) WITHOUT ROWID;
CREATE INDEX justification_span_by_evidence ON justification_span (evidence_id);
CREATE TABLE occurrence (
    id TEXT PRIMARY KEY, justification_id TEXT NOT NULL REFERENCES justification(id),
    source TEXT NOT NULL, scope TEXT NOT NULL, location TEXT NOT NULL, field TEXT NOT NULL,
    captured_at TEXT NOT NULL, source_updated_at TEXT, portable_key TEXT NOT NULL,
    evidence_id INTEGER NOT NULL REFERENCES evidence(id),
    end TEXT, end_evidence_id INTEGER REFERENCES evidence(id),
    CHECK ((end IS NULL) = (end_evidence_id IS NULL))
);
CREATE INDEX occurrence_by_justification ON occurrence (justification_id, captured_at, evidence_id);
CREATE INDEX occurrence_by_slot ON occurrence (source, scope, location, field, captured_at, evidence_id);
CREATE TABLE claim_link (
    from_claim TEXT NOT NULL REFERENCES claim(id), to_claim TEXT NOT NULL REFERENCES claim(id),
    kind TEXT NOT NULL CHECK (kind IN ('conflicts_with', 'supersedes')),
    PRIMARY KEY (from_claim, to_claim, kind)
) WITHOUT ROWID;
CREATE TABLE extraction_checkpoint (
    extractor TEXT NOT NULL, extractor_version TEXT NOT NULL, source TEXT NOT NULL,
    scope TEXT NOT NULL, last_captured_at TEXT NOT NULL, last_portable_key TEXT NOT NULL,
    last_evidence_id INTEGER NOT NULL, vocabulary_version TEXT NOT NULL,
    PRIMARY KEY (extractor, extractor_version, source, scope)
) WITHOUT ROWID;
-- This log is precious and is never included in a projection swap.
CREATE TABLE projection_publication (
    id INTEGER PRIMARY KEY, published_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    extractor_versions TEXT NOT NULL, vocabulary_versions TEXT NOT NULL, evidence_high_water TEXT NOT NULL
);
CREATE TRIGGER projection_publication_no_update BEFORE UPDATE ON projection_publication
BEGIN SELECT RAISE(ABORT, 'projection publications are insert-only'); END;
CREATE TRIGGER projection_publication_no_delete BEFORE DELETE ON projection_publication
BEGIN SELECT RAISE(ABORT, 'projection publications are insert-only'); END;
CREATE TRIGGER projection_publication_no_replace BEFORE INSERT ON projection_publication
WHEN EXISTS (SELECT 1 FROM projection_publication WHERE id = NEW.id)
BEGIN SELECT RAISE(ABORT, 'projection publications are insert-only'); END;
