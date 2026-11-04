-- A run records the window it was bound to, so a run that never finished can still say
-- what it failed to look at.
ALTER TABLE runs ADD COLUMN window_start TEXT;
ALTER TABLE runs ADD COLUMN window_end TEXT;
