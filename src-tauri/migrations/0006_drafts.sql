-- Drafts. docs/01 §6, docs/06 Phase 7 — *autosave every 30s and on blur, IMAP APPEND to the
-- Drafts mailbox so other devices see them.*
--
-- Its own table rather than a row in `message`, and the reason is what a draft *is*: a message
-- that does not exist yet. It has no UID until the server has been told, no `date_received`, no
-- flags worth speaking of, and it changes every thirty seconds. Putting that in `message` would
-- mean every list query, every FTS trigger and every sync pass had to know about a row that is
-- none of the things they assume a message to be.
CREATE TABLE draft (
  id          INTEGER PRIMARY KEY,
  account_id  INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,

  -- The UID the last APPEND landed on, if the server supports UIDPLUS and told us. Kept so the
  -- next save can delete the copy it replaces: without it, thirty seconds of typing produces
  -- one draft per save in every other client the user owns.
  remote_uid  INTEGER,

  -- Stable across saves, so the copies on the server can be recognised as the same draft even
  -- when the UID is unknown.
  message_id  TEXT NOT NULL,

  to_json     TEXT NOT NULL DEFAULT '[]',
  cc_json     TEXT NOT NULL DEFAULT '[]',
  bcc_json    TEXT NOT NULL DEFAULT '[]',
  subject     TEXT NOT NULL DEFAULT '',
  html        TEXT NOT NULL DEFAULT '',
  text        TEXT NOT NULL DEFAULT '',

  -- Threading, so a draft reply still threads when it is eventually sent.
  in_reply_to TEXT,
  references_ TEXT,

  updated_at  INTEGER NOT NULL
);

CREATE INDEX ix_draft_account ON draft(account_id, updated_at DESC);
CREATE UNIQUE INDEX ix_draft_message_id ON draft(message_id);
