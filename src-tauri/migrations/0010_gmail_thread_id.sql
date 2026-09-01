-- Gmail's conversation id, so threading can use the value that actually names a conversation.
--
-- `X-GM-THRID` was fetched from the first sync Phase 5 ever ran and then dropped on the floor:
-- there was no column to put it in. `rethread` filled the gap with `gm_msgid` instead, and left a
-- comment saying the fetch stored the thread id "separately", which it did not.
--
-- X-GM-MSGID is unique per message. X-GM-THRID is shared by every message in a conversation. Since
-- threading treats a Gmail thread id as authoritative and overriding, substituting the message id
-- gave every message a thread of its own and disabled threading entirely for Gmail accounts —
-- 1,584 messages across 1,483 threads on the account this was found on, and a reply whose
-- In-Reply-To and References both named its parent still filed on its own.
--
-- Existing rows get NULL, which is the right answer: threading falls back to the JWZ pass over
-- References for them until the next sync fetches the real value. `rethread` runs over the whole
-- account rather than the batch, so that recovery happens on its own.
ALTER TABLE message ADD COLUMN gm_thrid INTEGER;

CREATE INDEX IF NOT EXISTS message_gm_thrid ON message (account_id, gm_thrid)
    WHERE gm_thrid IS NOT NULL;

-- And rebuild the threads that were computed from the wrong key.
--
-- Adding the column fixes what happens next; it does nothing for the mail already stored, whose
-- `thread_id` values were assigned by reading a per-message id as a conversation id. Those rows
-- would keep their broken threads indefinitely, because `rethread` runs when a batch arrives or
-- when something is unthreaded, and neither is true of a mailbox that has finished syncing.
--
-- Setting `thread_id` to NULL is exactly the signal the engine already watches for:
-- `unthreaded_count` becomes non-zero, and the next sync runs a full rethread over the account.
-- One pass, on one sync, using machinery that already exists for the case of a first sync.
UPDATE message SET thread_id = NULL;

-- The rows nothing points at any more. `rethread` creates thread rows as it needs them, and the
-- ones left behind by the old assignment are why this account had more threads than messages.
DELETE FROM thread WHERE id NOT IN (SELECT thread_id FROM message WHERE thread_id IS NOT NULL);
