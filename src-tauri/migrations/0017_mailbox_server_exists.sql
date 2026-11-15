-- How many messages the server said a mailbox held, the last time it was checked.
--
-- A mailbox held only in part here — every folder but the Inbox past its newest 200 — could never
-- show more messages here than on the server, which was the only test for deletions, so a message
-- deleted elsewhere stayed here for ever as a row that opened blank (H2, 2026-11-14 audit). Those
-- mailboxes are now checked over the range they hold, whenever this count or UIDNEXT has moved.
-- NULL until the first check, so every mailbox is checked once after the upgrade.
ALTER TABLE mailbox ADD COLUMN server_exists INTEGER;
