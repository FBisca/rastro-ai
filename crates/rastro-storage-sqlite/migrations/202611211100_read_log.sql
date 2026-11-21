-- What an MCP read call asked (decisions/learning.md §17, the learning plan's read log): one row
-- per read of the store, insert-only and outside the rebuildable projection, because it is what a
-- machine read rather than what the owner saw. It is never a shown report: automation reading an
-- item must not make it look seen. The tool and its arguments name the call; the time dates it.
CREATE TABLE read_log (
    append_seq INTEGER PRIMARY KEY,
    tool TEXT NOT NULL CHECK (length(tool) > 0),
    arguments_json TEXT NOT NULL,
    at TEXT NOT NULL CHECK (length(at) > 0)
) STRICT;
CREATE TRIGGER read_log_no_update BEFORE UPDATE ON read_log
BEGIN SELECT RAISE(ABORT, 'the read log is insert-only'); END;
CREATE TRIGGER read_log_no_delete BEFORE DELETE ON read_log
BEGIN SELECT RAISE(ABORT, 'the read log is insert-only'); END;
CREATE TRIGGER read_log_no_replace BEFORE INSERT ON read_log
WHEN EXISTS (SELECT 1 FROM read_log WHERE append_seq = NEW.append_seq)
BEGIN SELECT RAISE(ABORT, 'the read log is insert-only'); END;