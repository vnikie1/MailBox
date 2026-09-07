-- Where the server still thinks a locally-moved message is.
--
-- `move_to` parks a moved row at a negative UID, because a UID belongs to one mailbox and
-- means nothing in another. That was right, and it threw away the one thing the row still
-- needed: until the queued operation reaches the server, the message *is* still in its old
-- mailbox under its old UID, and nothing local remembered where.
--
-- The cost was silent and came in two shapes.
--
-- `ops::locate` skips non-positive UIDs, correctly — there is no server UID to name — so every
-- command issued against a parked message queued **nothing at all**. Moving three messages
-- where one had just been moved queued an operation naming two of them; the third moved here
-- and never on the server. Flagging one did nothing. The local half worked, so it looked done.
--
-- And moving a parked message a second time left the first operation queued and unchanged, so
-- the server performed the first move and never the second. The returning message then failed
-- to match the parked row — `persist` adopts on `mailbox_id`, and the row had since moved
-- somewhere else — and was inserted beside it. One message, two rows, and `remove_missing`
-- will not clean up the parked one because it ignores `uid <= 0`.
--
-- Both are fixed by remembering the origin. It is cleared when the row rejoins the server's
-- numbering, so a non-null `origin_uid` means exactly "there is an unfinished move on this
-- row", which is also what lets a second move supersede the first rather than race it.
ALTER TABLE message ADD COLUMN origin_mailbox_id INTEGER;
ALTER TABLE message ADD COLUMN origin_uid INTEGER;

-- Rows parked by an earlier version have no origin to recover: the UID is gone. They keep the
-- old behaviour — skipped by `locate`, adopted if they happen to come back to the same mailbox
-- — which is no worse than before and cannot be improved after the fact.
CREATE INDEX IF NOT EXISTS message_origin ON message (origin_uid) WHERE origin_uid IS NOT NULL;
