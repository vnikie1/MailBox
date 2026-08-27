-- Search history. docs/06 Phase 9.
--
-- Keyed by the text itself rather than by an id, so running the same search again updates the
-- one row instead of adding a duplicate the user then has to scroll past.
--
-- Local, like everything else here. A list of what somebody has searched their own mail for is
-- among the most revealing things this app holds, and standing rule 16 means it never leaves
-- the machine — but it is worth saying explicitly in the place it is stored.
CREATE TABLE search_history (
  text       TEXT PRIMARY KEY,
  last_used  INTEGER NOT NULL,
  times_used INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX ix_search_history_recent ON search_history(last_used DESC);
