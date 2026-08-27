-- Phase 8: VIPs, Remind Me, Follow Up, and named flags. docs/01 §8.

-- VIPs are addresses, not contacts, because that is all the mail stream gives us. Mail caps
-- the list at 100; the cap is enforced in code rather than here, so hitting it can produce a
-- message the user understands instead of a constraint violation.
CREATE TABLE vip (
  address    TEXT PRIMARY KEY,          -- lower-cased on the way in; see rules::vip
  added_at   INTEGER NOT NULL
);

-- Flag names. The seven colours are fixed; their labels are not — docs/01 §8 calls them
-- renameable. Absent row means the colour uses its default name.
CREATE TABLE flag_name (
  color      TEXT PRIMARY KEY,          -- red|orange|yellow|green|blue|purple|gray
  name       TEXT NOT NULL
);

-- Remind Me already has its column and index in 0001; only Follow Up is new here.

-- Follow Up: a sent message that asked a question and has had no reply. Set by the detector,
-- cleared when a reply arrives or the user dismisses it.
ALTER TABLE message ADD COLUMN follow_up_at INTEGER;

-- Junk scoring, kept beside the flag so a reclassification can tell "the filter thought so"
-- from "the user said so". Without that distinction, training on our own output teaches the
-- classifier its own mistakes.
ALTER TABLE message ADD COLUMN junk_score REAL;
ALTER TABLE message ADD COLUMN junk_by_user INTEGER NOT NULL DEFAULT 0;

-- The classifier's corpus. One row per token per class, so training is an upsert and
-- scoring is a lookup. Kept local and never transmitted (standing rule 16).
CREATE TABLE junk_token (
  token      TEXT NOT NULL,
  is_junk    INTEGER NOT NULL,
  count      INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (token, is_junk)
);

CREATE TABLE junk_corpus (
  is_junk    INTEGER PRIMARY KEY,
  messages   INTEGER NOT NULL DEFAULT 0
);

-- Blocked senders. Mail's "Block Sender" is a rule in disguise, but it needs to be one click
-- and survive rule edits, so it gets its own table.
CREATE TABLE blocked_sender (
  address    TEXT PRIMARY KEY,
  blocked_at INTEGER NOT NULL
);

-- Read on every list query for the Inbox, so it needs to be cheap. The matching snooze index
-- is already in 0001.
CREATE INDEX ix_msg_follow_up ON message(follow_up_at) WHERE follow_up_at IS NOT NULL;
