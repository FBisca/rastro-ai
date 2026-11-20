-- Turns belong to a capped session, whatever the task (ADR 0016 §1): the history table takes the
-- session's name. The rename keeps every recorded turn and its meeting_run reference; an export
-- still leaves the history out, and forgetting the meeting still removes it.
ALTER TABLE judgment_turn RENAME TO session_turn;
