CREATE TABLE automatic_justification (
    id TEXT PRIMARY KEY, claim_id TEXT NOT NULL, reason TEXT NOT NULL,
    premise_a TEXT NOT NULL, premise_b TEXT,
    started_at TEXT NOT NULL, ended_at TEXT
);
CREATE INDEX automatic_justification_by_claim ON automatic_justification (claim_id);
