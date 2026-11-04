-- Which observations a run saw: new ones it appended and unchanged ones it re-confirmed.
-- Evidence itself stays insert-only; this table is the link.
CREATE TABLE run_evidence (
    run_id INTEGER NOT NULL REFERENCES runs(id),
    evidence_id INTEGER NOT NULL REFERENCES evidence(id),
    PRIMARY KEY (run_id, evidence_id)
) WITHOUT ROWID;
CREATE INDEX evidence_latest_by_location ON evidence (source, scope, location, field, id);
