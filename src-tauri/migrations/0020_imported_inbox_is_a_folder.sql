-- An imported folder named Inbox is a folder, not an inbox. L40, 2026-11-14 audit.
--
-- Imports gave a folder called "Inbox" the inbox role, as they give "Sent Items" the sent role.
-- The inbox role is not only an icon: the taskbar badge counts unread mail in every inbox, and the
-- Inboxes row lists every inbox's mail. So years-old unread mail from an archive joined the badge
-- and the Inboxes row, and stayed there. Imports now leave the role off; this takes it off the
-- folders earlier imports made. Imported mail lives under the one local account, which has no
-- server and so no inbox of its own.

UPDATE mailbox
   SET role = NULL
 WHERE role = 'inbox'
   AND account_id IN (SELECT id FROM account WHERE provider = 'local');
