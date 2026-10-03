-- Cached message sources that no row points at any more, waiting to be deleted.
--
-- A downloaded message is kept twice: parsed into its row, and as its raw source in
-- `bodies/<account>/<message>.eml`. Nothing deleted the second copy. Removing an account purged
-- its rows, its search entries and its password and left every message it had downloaded on
-- disk; Erase Deleted Items did the same to each message it erased. See `sync::sources`.
--
-- Fed by triggers rather than by the code that deletes, for the reason the search index is:
-- messages are deleted in four places (account removal, mailbox deletion, the batch delete in
-- `db::write`, and sync's expunge and reset handling), and a fifth will not have to remember.
CREATE TABLE discarded_source (
  path TEXT PRIMARY KEY NOT NULL
) WITHOUT ROWID;

CREATE TRIGGER message_source_discarded AFTER DELETE ON message
WHEN old.raw_path IS NOT NULL
BEGIN
  INSERT OR IGNORE INTO discarded_source (path) VALUES (old.raw_path);
END;

-- A row that stops pointing at a file lets it go just as a deleted one does. A body fetched
-- again rewrites the same path, so the usual update leaves the queue alone.
CREATE TRIGGER message_source_replaced AFTER UPDATE OF raw_path ON message
WHEN old.raw_path IS NOT NULL AND old.raw_path IS NOT new.raw_path
BEGIN
  INSERT OR IGNORE INTO discarded_source (path) VALUES (old.raw_path);
END;
