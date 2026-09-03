-- Fills the contact index from the mail already in the store.
--
-- `contact` has existed since the first migration and `search::suggest` has read it since Phase
-- 9, but nothing ever wrote it -- so the People group of the suggestion list could not appear
-- for anybody, ever. `persist::write_batch` records senders now, which covers every message from
-- here on.
--
-- That is not enough on its own. An install with 50,000 messages already downloaded would have
-- an empty index and would stay effectively empty for weeks, until enough new mail arrived to
-- make the suggestions useful. The people worth suggesting are the ones already in the mailbox.
--
-- Runs once, as part of the migration that introduces the behaviour, rather than as a scan on
-- every start.
--
-- `seen_count` is the number of messages from that address, which is what orders the
-- suggestions; `last_seen` is the newest, so a correspondent from six years ago does not outrank
-- one from this morning on count alone. The name is the most recent non-empty one -- people
-- change how they sign their mail, and the current form is the one worth offering.
INSERT INTO contact (addr, name, seen_count, last_seen)
SELECT LOWER(TRIM(message.from_addr)),
       (
         SELECT NULLIF(TRIM(COALESCE(newest.from_name, '')), '')
           FROM message AS newest
          WHERE LOWER(TRIM(newest.from_addr)) = LOWER(TRIM(message.from_addr))
            AND TRIM(COALESCE(newest.from_name, '')) <> ''
          ORDER BY newest.date_received DESC
          LIMIT 1
       ),
       COUNT(*),
       MAX(message.date_received)
  FROM message
 WHERE message.from_addr IS NOT NULL
   AND TRIM(message.from_addr) <> ''
 GROUP BY LOWER(TRIM(message.from_addr))
ON CONFLICT(addr) DO UPDATE SET
       name       = COALESCE(excluded.name, contact.name),
       seen_count = MAX(contact.seen_count, excluded.seen_count),
       last_seen  = MAX(COALESCE(contact.last_seen, 0), excluded.last_seen);
