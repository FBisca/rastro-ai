-- What a meeting run retains so its results can be replayed without a model call, and the opt-in
-- full transcript. Only the latest good run keeps them; forgetting a meeting deletes them all.
ALTER TABLE meeting_run ADD COLUMN replay_of INTEGER;
-- The candidates the extractor proposed that the core kept: the recorded model output.
CREATE TABLE meeting_output (
    run_id INTEGER PRIMARY KEY REFERENCES meeting_run (id),
    candidates TEXT NOT NULL
) STRICT;
-- The turns those candidates cite (their text), and the turns next to them (speaker only, empty
-- text): the supporting spans.
CREATE TABLE meeting_span (
    run_id INTEGER NOT NULL REFERENCES meeting_run (id),
    turn INTEGER NOT NULL,
    speaker TEXT NOT NULL,
    text TEXT NOT NULL,
    PRIMARY KEY (run_id, turn)
) STRICT;
-- The explicit speaker mappings the run used, the owner's included.
CREATE TABLE meeting_speaker (
    run_id INTEGER NOT NULL REFERENCES meeting_run (id),
    label TEXT NOT NULL,
    entity TEXT NOT NULL,
    identifier TEXT NOT NULL,
    PRIMARY KEY (run_id, label)
) STRICT;
-- The full transcript, only when the owner opted in at import.
CREATE TABLE meeting_transcript (
    meeting_id TEXT PRIMARY KEY REFERENCES meeting (id),
    run_id INTEGER NOT NULL,
    text TEXT NOT NULL
) STRICT;
