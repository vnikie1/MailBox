-- Halcyon initial schema. docs/03-architecture.md §3.
--
-- Migrations are forward-only, so this file is written once and never edited again: any
-- change lands as 0002. That is why the whole schema is here rather than only the tables
-- Phase 3 queries — the shape is specified, and adding a column later costs a migration
-- while getting it right now costs nothing.
--
-- Two deliberate departures from §3, both explained where they occur: the FTS5 table is
-- external-content rather than contentless, and `message` carries three denormalised
-- columns to feed it.

-- ---------------------------------------------------------------- accounts

CREATE TABLE account (
  id            INTEGER PRIMARY KEY,
  display_name  TEXT NOT NULL,
  email         TEXT NOT NULL UNIQUE,
  provider      TEXT NOT NULL,            -- gmail | outlook | icloud | imap
  imap_host     TEXT,
  imap_port     INTEGER,
  imap_security TEXT,
  smtp_host     TEXT,
  smtp_port     INTEGER,
  smtp_security TEXT,
  auth_kind     TEXT NOT NULL,            -- oauth2 | password
  -- A Credential Manager key, never a secret. Standing rule 12.
  cred_ref      TEXT NOT NULL,
  color         TEXT,
  sort_order    INTEGER NOT NULL DEFAULT 0,
  sync_enabled  INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE mailbox (
  id             INTEGER PRIMARY KEY,
  account_id     INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  remote_path    TEXT NOT NULL,           -- IMAP path, raw
  display_name   TEXT NOT NULL,
  parent_id      INTEGER REFERENCES mailbox(id),
  role           TEXT,                    -- inbox|drafts|sent|junk|trash|archive|all|null
  uid_validity   INTEGER,
  uid_next       INTEGER,
  highest_modseq INTEGER,                 -- CONDSTORE
  unread_count   INTEGER NOT NULL DEFAULT 0,
  total_count    INTEGER NOT NULL DEFAULT 0,
  sort_order     INTEGER NOT NULL DEFAULT 0,
  subscribed     INTEGER NOT NULL DEFAULT 1,
  UNIQUE(account_id, remote_path)
);

CREATE INDEX ix_mailbox_account ON mailbox(account_id, sort_order);

-- ---------------------------------------------------------------- messages

CREATE TABLE thread (
  id            INTEGER PRIMARY KEY,
  account_id    INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  subject_base  TEXT,
  last_date     INTEGER,
  message_count INTEGER NOT NULL DEFAULT 1,
  unread_count  INTEGER NOT NULL DEFAULT 0,
  muted         INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE message (
  id            INTEGER PRIMARY KEY,
  account_id    INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  mailbox_id    INTEGER NOT NULL REFERENCES mailbox(id) ON DELETE CASCADE,
  uid           INTEGER NOT NULL,
  message_id    TEXT,                     -- RFC Message-ID header
  thread_id     INTEGER REFERENCES thread(id),
  in_reply_to   TEXT,
  references_   TEXT,                     -- space-joined
  gm_msgid      TEXT,                     -- Gmail X-GM-MSGID, null elsewhere

  subject       TEXT,
  subject_base  TEXT,                     -- Re:/Fwd: stripped, for threading
  from_name     TEXT,
  from_addr     TEXT,
  to_json       TEXT,
  cc_json       TEXT,
  bcc_json      TEXT,
  reply_to_json TEXT,

  date_sent     INTEGER NOT NULL,         -- epoch seconds, UTC
  date_received INTEGER NOT NULL,
  size          INTEGER NOT NULL DEFAULT 0,
  preview       TEXT,                     -- cleaned, ~300 chars

  flag_seen      INTEGER NOT NULL DEFAULT 0,
  flag_answered  INTEGER NOT NULL DEFAULT 0,
  flag_flagged   INTEGER NOT NULL DEFAULT 0,
  flag_draft     INTEGER NOT NULL DEFAULT 0,
  flag_deleted   INTEGER NOT NULL DEFAULT 0,
  flag_color     TEXT,                    -- red|orange|yellow|green|blue|purple|gray
  has_attachment INTEGER NOT NULL DEFAULT 0,
  is_junk        INTEGER NOT NULL DEFAULT 0,
  snooze_until   INTEGER,

  body_state    TEXT NOT NULL DEFAULT 'none',  -- none|headers|full
  raw_path      TEXT,                     -- .eml on disk when body_state='full'
  body_html     TEXT,
  body_text     TEXT,

  -- Denormalised for the FTS index below. External-content FTS5 reads its columns from
  -- the content table by name, so anything searchable has to exist as a column here —
  -- these three are what §3's `from_all` / `to_all` / `attachment_names` become. The
  -- writer fills them; nothing else reads them.
  from_all         TEXT,
  to_all           TEXT,
  attachment_names TEXT,

  UNIQUE(mailbox_id, uid)
);

-- The list query. (mailbox_id, date_received DESC, id DESC) rather than §3's two-column
-- index: keyset pagination compares the pair (date_received, id), and without id in the
-- index that comparison cannot be answered from the index alone.
CREATE INDEX ix_msg_list ON message(mailbox_id, date_received DESC, id DESC);

CREATE INDEX ix_msg_thread ON message(thread_id, date_sent);
CREATE INDEX ix_msg_msgid ON message(message_id);
CREATE INDEX ix_msg_gm ON message(gm_msgid) WHERE gm_msgid IS NOT NULL;

-- Partial: an unread count scans only the unread rows, which at a healthy inbox is a few
-- hundred out of a hundred thousand.
CREATE INDEX ix_msg_unread ON message(mailbox_id) WHERE flag_seen = 0;

CREATE INDEX ix_msg_snooze ON message(snooze_until) WHERE snooze_until IS NOT NULL;

CREATE TABLE attachment (
  id         INTEGER PRIMARY KEY,
  message_id INTEGER NOT NULL REFERENCES message(id) ON DELETE CASCADE,
  part_id    TEXT,
  filename   TEXT,
  mime       TEXT,
  size       INTEGER,
  content_id TEXT,
  is_inline  INTEGER NOT NULL DEFAULT 0,
  cache_path TEXT
);

CREATE INDEX ix_attachment_message ON attachment(message_id);

-- ---------------------------------------------------------------- search
--
-- External-content rather than §3's `content=''`.
--
-- docs/06 Phase 3 asks for "FTS5 external-content with triggers keeping it in sync", and
-- §3's contentless form contradicts that. External content wins for a concrete reason: a
-- contentless table cannot be updated in place, and deleting from one means re-supplying
-- every original column value in the DELETE trigger. Attachment names live in another
-- table and are not available in `OLD`, so that trigger cannot be written correctly. The
-- denormalised columns above make the external-content form exact instead.

CREATE VIRTUAL TABLE message_fts USING fts5(
  subject,
  body_text,
  from_all,
  to_all,
  attachment_names,
  content='message',
  content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER message_fts_insert AFTER INSERT ON message BEGIN
  INSERT INTO message_fts(rowid, subject, body_text, from_all, to_all, attachment_names)
  VALUES (new.id, new.subject, new.body_text, new.from_all, new.to_all, new.attachment_names);
END;

CREATE TRIGGER message_fts_delete AFTER DELETE ON message BEGIN
  INSERT INTO message_fts(message_fts, rowid, subject, body_text, from_all, to_all, attachment_names)
  VALUES ('delete', old.id, old.subject, old.body_text, old.from_all, old.to_all, old.attachment_names);
END;

CREATE TRIGGER message_fts_update AFTER UPDATE ON message BEGIN
  INSERT INTO message_fts(message_fts, rowid, subject, body_text, from_all, to_all, attachment_names)
  VALUES ('delete', old.id, old.subject, old.body_text, old.from_all, old.to_all, old.attachment_names);
  INSERT INTO message_fts(rowid, subject, body_text, from_all, to_all, attachment_names)
  VALUES (new.id, new.subject, new.body_text, new.from_all, new.to_all, new.attachment_names);
END;

-- ---------------------------------------------------------------- people and rules

CREATE TABLE contact (
  id          INTEGER PRIMARY KEY,
  addr        TEXT NOT NULL UNIQUE,
  name        TEXT,
  avatar_path TEXT,
  is_vip      INTEGER NOT NULL DEFAULT 0,
  is_blocked  INTEGER NOT NULL DEFAULT 0,
  seen_count  INTEGER NOT NULL DEFAULT 0,
  last_seen   INTEGER
);

CREATE TABLE smart_mailbox (
  id             INTEGER PRIMARY KEY,
  name           TEXT NOT NULL,
  icon           TEXT,
  match_all      INTEGER NOT NULL DEFAULT 1,
  predicate_json TEXT NOT NULL,
  sort_order     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE rule (
  id             INTEGER PRIMARY KEY,
  name           TEXT NOT NULL,
  enabled        INTEGER NOT NULL DEFAULT 1,
  match_all      INTEGER NOT NULL DEFAULT 1,
  predicate_json TEXT NOT NULL,
  actions_json   TEXT NOT NULL,
  sort_order     INTEGER NOT NULL DEFAULT 0
);

-- ---------------------------------------------------------------- outbound and sync
--
-- The durable outbox is what makes Undo Send and offline sending correct, and pending_op
-- is the mechanism behind "deleting is 0ms" — the UI mutates `message`, inserts an op and
-- returns, and a worker drains the ops with backoff. Standing rule 10.

CREATE TABLE outbox (
  id         INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  eml_path   TEXT NOT NULL,
  state      TEXT NOT NULL,               -- holding|queued|sending|sent|failed
  send_after INTEGER NOT NULL,            -- undo-send / send-later timestamp
  attempts   INTEGER NOT NULL DEFAULT 0,
  last_error TEXT
);

CREATE INDEX ix_outbox_due ON outbox(state, send_after);

CREATE TABLE pending_op (
  id           INTEGER PRIMARY KEY,
  account_id   INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  kind         TEXT NOT NULL,             -- flag|move|copy|delete|expunge|append
  payload_json TEXT NOT NULL,
  created_at   INTEGER NOT NULL,
  attempts     INTEGER NOT NULL DEFAULT 0,
  last_error   TEXT
);

CREATE INDEX ix_pending_op_account ON pending_op(account_id, id);

-- ---------------------------------------------------------------- settings

CREATE TABLE setting (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
