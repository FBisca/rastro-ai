-- foreign_keys = off
-- A claim can read `confirmed` or `retracted` as well as `observed` or `superseded` (ADR 0013).
-- SQLite can't change a CHECK in place, so the table is rebuilt the way its documentation says:
-- the open runs this migration with foreign keys off, then checks them before it commits. Every
-- claim and every row that names one is kept, with its status; the projection stays rebuildable.
CREATE TABLE claim_widened (
    id TEXT PRIMARY KEY, subject TEXT NOT NULL REFERENCES entity(id), predicate TEXT NOT NULL,
    object_entity TEXT REFERENCES entity(id), object_value TEXT, object_datatype TEXT,
    scalar INTEGER NOT NULL CHECK (scalar IN (0, 1)),
    status TEXT NOT NULL CHECK (status IN ('observed', 'confirmed', 'retracted', 'superseded')),
    revision TEXT NOT NULL,
    known_since TEXT NOT NULL DEFAULT '',
    type_ancestors TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(type_ancestors) AND json_type(type_ancestors) = 'array'),
    CHECK ((object_entity IS NULL) = (object_value IS NOT NULL AND object_datatype IS NOT NULL))
);
INSERT INTO claim_widened (id, subject, predicate, object_entity, object_value, object_datatype,
    scalar, status, revision, known_since, type_ancestors)
SELECT id, subject, predicate, object_entity, object_value, object_datatype,
    scalar, status, revision, known_since, type_ancestors FROM claim;
DROP TABLE claim;
ALTER TABLE claim_widened RENAME TO claim;
CREATE INDEX claim_by_subject ON claim (subject, known_since DESC, id);
CREATE INDEX claim_by_object ON claim (object_entity, known_since DESC, id);
CREATE INDEX claim_alias ON claim (predicate, object_value) WHERE predicate = 'rastro:alias';
