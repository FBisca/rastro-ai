-- What a terminal queue read showed (decisions/learning.md §3): one report per item it printed,
-- insert-only and outside the rebuildable projection, because it is what the owner saw rather
-- than what Rastro knows. The source and its run let an outcome date the item's departure.
CREATE TABLE shown_report (
    append_seq INTEGER PRIMARY KEY,
    item TEXT NOT NULL CHECK (length(item) > 0),
    source TEXT NOT NULL CHECK (length(source) > 0),
    run TEXT NOT NULL,
    shown_at TEXT NOT NULL CHECK (length(shown_at) > 0)
) STRICT;
CREATE INDEX shown_report_by_item ON shown_report (item, append_seq);
CREATE TRIGGER shown_report_no_update BEFORE UPDATE ON shown_report
BEGIN SELECT RAISE(ABORT, 'shown reports are insert-only'); END;
CREATE TRIGGER shown_report_no_delete BEFORE DELETE ON shown_report
BEGIN SELECT RAISE(ABORT, 'shown reports are insert-only'); END;
CREATE TRIGGER shown_report_no_replace BEFORE INSERT ON shown_report
WHEN EXISTS (SELECT 1 FROM shown_report WHERE append_seq = NEW.append_seq)
BEGIN SELECT RAISE(ABORT, 'shown reports are insert-only'); END;
