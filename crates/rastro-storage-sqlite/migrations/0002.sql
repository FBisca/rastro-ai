CREATE TRIGGER evidence_no_replace BEFORE INSERT ON evidence
WHEN EXISTS (SELECT 1 FROM evidence WHERE id = NEW.id)
BEGIN SELECT RAISE(ABORT, 'evidence is insert-only'); END;
