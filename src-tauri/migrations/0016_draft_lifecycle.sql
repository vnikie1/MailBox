-- A draft's own attachments, and the outbox message a draft became.
--
-- Drafts never kept attachments: a message saved with a file on it came back without one, if it
-- came back at all (C3, 2026-11-14 audit). The paths are this machine's; the server copy carries
-- the files themselves, for a draft reopened anywhere else.
ALTER TABLE draft ADD COLUMN attachments_json TEXT NOT NULL DEFAULT '[]';

-- Which draft a queued message was written as, so the draft is discarded when the message has
-- actually gone — not when Send was pressed. Sending used to leave the draft behind for ever, on
-- the server too, so a sent message kept reappearing in Drafts on every device (H4). Discarding it
-- at Send instead would lose the message on Undo Send, which deletes the outbox copy.
ALTER TABLE outbox ADD COLUMN draft_message_id TEXT;
