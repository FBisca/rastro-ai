-- The standard statements each printed item expressed, as a JSON array of {predicate, value}.
-- They are what decides whether the queue later left the item out as already known, kept with
-- the report because the item is gone by the time an outcome is read: an item the queue settled
-- elsewhere stays became moot even after a later run's content no longer carries it. Null for a
-- report recorded before this column existed, and an empty array for a question.
ALTER TABLE shown_report ADD COLUMN statements TEXT;
