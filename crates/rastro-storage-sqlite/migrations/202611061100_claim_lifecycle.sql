ALTER TABLE claim ADD COLUMN scalar INTEGER NOT NULL DEFAULT 0 CHECK (scalar IN (0, 1));
ALTER TABLE claim ADD COLUMN valid_time TEXT NOT NULL DEFAULT 'unknown' CHECK (valid_time = 'unknown');
CREATE TABLE known_interval (
    claim_id TEXT NOT NULL REFERENCES claim(id), evidence_id INTEGER NOT NULL REFERENCES evidence(id),
    start TEXT NOT NULL, end TEXT,
    PRIMARY KEY (claim_id, evidence_id)
) WITHOUT ROWID;
CREATE TABLE claim_link (
    from_claim TEXT NOT NULL REFERENCES claim(id), to_claim TEXT NOT NULL REFERENCES claim(id),
    kind TEXT NOT NULL CHECK (kind IN ('conflicts_with', 'supersedes')),
    PRIMARY KEY (from_claim, to_claim, kind)
) WITHOUT ROWID;
CREATE TABLE projection_generation (id INTEGER NOT NULL);
INSERT INTO projection_generation VALUES (1);
