-- Every turn of a run's judgment session (ADR 0016 §3): the request the core sent, the answer
-- the adapter gave (NULL when it never answered) and the core's gate verdict. Kept for every
-- run, failed ones included; an export leaves it out, and forgetting the meeting removes it.
CREATE TABLE judgment_turn (
    run_id INTEGER NOT NULL REFERENCES meeting_run (id),
    turn INTEGER NOT NULL CHECK (turn > 0),
    request TEXT NOT NULL CHECK (length(request) > 0),
    answer TEXT,
    verdict TEXT NOT NULL CHECK (length(verdict) > 0),
    PRIMARY KEY (run_id, turn)
) STRICT;