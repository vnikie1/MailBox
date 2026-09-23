-- The rest of the mailbox menu: Favourites in one order the user can drag, Use This Mailbox As,
-- and Rebuild.

-- Favourites, in the order the sidebar shows them: the rows every sidebar starts with and the
-- mailboxes the user added, in one list.
--
-- One list because docs/01 §3 makes the whole section reorderable by drag, and a mailbox dragged
-- above Flagged has to stay there. 0013 kept a position on the mailbox row, which could order the
-- user's mailboxes among themselves and never against the built-in rows, which had no row at all.
--
-- A mailbox's entry goes with the mailbox (ON DELETE CASCADE), which is what the column did for
-- free and a separate table has to say. A built-in row is named rather than numbered, by the key
-- the window builds it under.
CREATE TABLE favourite (
  id          INTEGER PRIMARY KEY,
  position    INTEGER NOT NULL,
  builtin     TEXT UNIQUE,
  mailbox_id  INTEGER UNIQUE REFERENCES mailbox(id) ON DELETE CASCADE,
  CHECK ((builtin IS NULL) <> (mailbox_id IS NULL))
);

CREATE INDEX ix_favourite_position ON favourite(position);

-- The order the sidebar has always drawn them in.
INSERT INTO favourite (position, builtin) VALUES
  (1, 'allInboxes'),
  (2, 'vips'),
  (3, 'flagged'),
  (4, 'allDrafts'),
  (5, 'allSent');

-- Then the mailboxes already added, after them and in the order they were added, which is where
-- the sidebar showed them.
INSERT INTO favourite (position, mailbox_id)
  SELECT 5 + ROW_NUMBER() OVER (ORDER BY favourite_order, id), id
    FROM mailbox
   WHERE favourite_order IS NOT NULL;

ALTER TABLE mailbox DROP COLUMN favourite_order;

-- Use This Mailbox As: the mailbox the user chose for a role, over what the server said.
--
-- Its own table because `sync::mailboxes::persist` rewrites `mailbox.role` from the server on
-- every sync, and a choice stored there would last until the next one. One mailbox per role per
-- account, and one role per mailbox. Goes with the mailbox, and with the account.
CREATE TABLE mailbox_role (
  account_id  INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  role        TEXT NOT NULL,
  mailbox_id  INTEGER NOT NULL UNIQUE REFERENCES mailbox(id) ON DELETE CASCADE,
  PRIMARY KEY (account_id, role)
);

-- Rebuild: set when the user asks, cleared by the sync that has read the whole mailbox again.
--
-- A flag rather than work done by the command, because the rebuild has to wait for the changes
-- queued before it: a message deleted here and not yet on the server would otherwise be read
-- back from the server and reappear.
ALTER TABLE mailbox ADD COLUMN rebuild_requested INTEGER NOT NULL DEFAULT 0;
