-- Meetings move into the one conversation store every kind shares (ADR 0011, amendment
-- 2026-11-22): the eight meeting tables take the conversation name, a kind column says what each
-- conversation is, and every existing meeting keeps its rows, keys and results unchanged. A
-- meeting imported before the move reads the same hypotheses, stale flag and coverage after it,
-- and replays with no adapter call.
ALTER TABLE meeting RENAME TO conversation;
ALTER TABLE conversation ADD COLUMN kind TEXT NOT NULL DEFAULT 'meeting'
    CHECK (kind IN ('meeting', 'chat'));
ALTER TABLE meeting_run RENAME TO conversation_run;
ALTER TABLE conversation_run RENAME COLUMN meeting_id TO conversation_id;
ALTER TABLE meeting_hypothesis RENAME TO conversation_hypothesis;
ALTER TABLE meeting_output RENAME TO conversation_output;
ALTER TABLE meeting_span RENAME TO conversation_span;
ALTER TABLE meeting_speaker RENAME TO conversation_speaker;
ALTER TABLE meeting_contract RENAME TO conversation_contract;
ALTER TABLE meeting_transcript RENAME TO conversation_transcript;
ALTER TABLE conversation_transcript RENAME COLUMN meeting_id TO conversation_id;
DROP INDEX meeting_run_by_meeting;
CREATE INDEX conversation_run_by_conversation ON conversation_run (conversation_id, id);
