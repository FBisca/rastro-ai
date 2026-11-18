-- The queue items a successful run's content proposed, as a JSON array of content addresses.
-- Only the latest good run keeps its hypotheses, so an outcome reads this to date the run that
-- dropped an item (decisions/learning.md §18). Null for a failed run, and for a run recorded
-- before this column existed.
ALTER TABLE meeting_run ADD COLUMN items TEXT;
