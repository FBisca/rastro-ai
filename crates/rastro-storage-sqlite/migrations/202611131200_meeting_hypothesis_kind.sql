-- What a hypothesis is: an ask, a promise or a waiting-on. A hypothesis recorded before adapters
-- proposed kinds is read as a promise.
ALTER TABLE meeting_hypothesis ADD COLUMN kind TEXT NOT NULL DEFAULT 'promise'
    CHECK (kind IN ('ask', 'promise', 'waiting_on'));
