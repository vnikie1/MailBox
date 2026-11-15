-- The UIDVALIDITY of the mailbox a queued change names by UID, as it was when the change was
-- queued.
--
-- A UID means nothing without it. A server that resets a mailbox gives the same numbers to
-- different messages, and a Delete queued before the reset expunged whichever message had the
-- number afterwards (M3, 2026-11-14 audit). The drain now compares this with what SELECT says,
-- and drops a change that names UIDs from before a reset instead of applying it to other mail.
-- NULL for changes queued before this column existed, and for the kinds that name no UIDs.
ALTER TABLE pending_op ADD COLUMN uid_validity INTEGER;
