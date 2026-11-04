-- Full-text index over evidence quotes: a projection, rebuilt from `evidence` at any time.
CREATE VIRTUAL TABLE evidence_fts USING fts5(
    quote, content='evidence', content_rowid='id',
    tokenize='porter unicode61 remove_diacritics 2'
);
INSERT INTO evidence_fts(evidence_fts) VALUES('rebuild');
