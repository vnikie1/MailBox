-- What the outbox needs to survive being killed mid-send. docs/04 Phase 7 exit gate:
-- *killing the app mid-send does not lose or duplicate the message.*
--
-- There is a window, however small, between SMTP accepting a message and this process
-- recording that it did. A crash inside that window leaves a row that says `sending` and no
-- way to know which side of the line it fell on. Both of the obvious answers are wrong:
-- retrying may send it twice, and giving up may lose it entirely, and both do so silently.
--
-- The way out is to ask the server. Every message this app sends carries a `Message-ID` we
-- generated, and a sent message lands in the account's Sent mailbox. On restart the outbox
-- searches Sent for that id — found means it went, absent means it did not — and the answer is
-- authoritative rather than a guess. That only works if the id is stored beside the row, which
-- is what this column is for.
ALTER TABLE outbox ADD COLUMN message_id TEXT;

-- The subject and recipient summary, so the outbox and its failure banner can describe a
-- message without parsing the `.eml` off disk to draw a list row.
ALTER TABLE outbox ADD COLUMN subject TEXT;
ALTER TABLE outbox ADD COLUMN recipients TEXT;

-- When the row was created, which is what "Undo Send" counts from and what the outbox list
-- orders by. `send_after` is a deadline and moves; this does not.
ALTER TABLE outbox ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0;

-- Looking a row up by the id the server would know it by, for the recovery above.
CREATE INDEX ix_outbox_message_id ON outbox(message_id) WHERE message_id IS NOT NULL;
