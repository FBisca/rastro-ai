-- Owner intents (ADR 0010): a precious input, insert-only, outside the rebuildable projection.
-- No foreign key into the projection; `undo_of` and `supersedes` refer to earlier intents.
-- No forget path yet: forgetting an intent waits for a controlled purge that leaves a marker.
CREATE TABLE intent (
    append_seq INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE CHECK (length(id) > 0),
    format_version INTEGER NOT NULL CHECK (format_version > 0),
    kind TEXT NOT NULL CHECK (length(kind) > 0),
    owner_id TEXT NOT NULL CHECK (length(owner_id) > 0),
    issuer_context TEXT NOT NULL CHECK (json_valid(issuer_context) AND json_type(issuer_context) = 'object'),
    issued_at TEXT NOT NULL CHECK (length(issued_at) > 0),
    domain_key TEXT NOT NULL CHECK (length(domain_key) > 0),
    targets_json TEXT NOT NULL CHECK (json_valid(targets_json) AND json_type(targets_json) = 'object'),
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json) AND json_type(payload_json) = 'object'),
    undo_of TEXT REFERENCES intent(id),
    supersedes TEXT REFERENCES intent(id),
    CHECK (undo_of IS NULL OR undo_of <> id),
    CHECK (supersedes IS NULL OR supersedes <> id)
);
CREATE INDEX intent_by_domain ON intent (domain_key, append_seq);
CREATE TRIGGER intent_no_update BEFORE UPDATE ON intent
BEGIN SELECT RAISE(ABORT, 'intents are insert-only'); END;
CREATE TRIGGER intent_no_delete BEFORE DELETE ON intent
BEGIN SELECT RAISE(ABORT, 'intents are insert-only'); END;
CREATE TRIGGER intent_no_replace BEFORE INSERT ON intent
WHEN EXISTS (SELECT 1 FROM intent WHERE append_seq = NEW.append_seq OR id = NEW.id)
BEGIN SELECT RAISE(ABORT, 'intents are insert-only'); END;
-- Which intent backs which claim: part of the disposable projection, derived from `intent`.
CREATE TABLE intent_justification (
    id TEXT PRIMARY KEY, claim_id TEXT NOT NULL REFERENCES claim(id), intent_id TEXT NOT NULL
);
CREATE INDEX intent_justification_by_claim ON intent_justification (claim_id);
