-- The mailbox context menu: New, Rename and Delete Mailbox, and Add to Favourites.

-- Where a mailbox sits in Favourites, or NULL when it is not there.
--
-- On the row rather than in the window's storage, for two reasons. A renamed folder keeps its
-- row, so it stays a favourite without anything having to follow the rename; and a deleted
-- folder takes its place in Favourites with it, where a list of ids kept elsewhere would go on
-- naming a mailbox that no longer exists — or, once SQLite reuses the id, a different one.
--
-- Ordered, and appended to, so adding a favourite never moves the ones already there: Ctrl+1 to
-- Ctrl+9 walk Favourites, and a shortcut that changes meaning when something is added is a trap.
ALTER TABLE mailbox ADD COLUMN favourite_order INTEGER;

-- The hierarchy separator the server listed this mailbox with: "/" for Gmail and most Dovecot
-- setups, "." for Courier and cPanel.
--
-- `LIST` has always returned it and `sync::mailboxes` has always read it, and nothing stored it,
-- because nothing needed it: the app only ever read folders. Making and renaming them means
-- building paths, and a name typed as "Q3/Q4" is a folder inside a folder on a server that uses
-- "/" and a single folder on one that does not. NULL until the next sync lists the mailbox.
ALTER TABLE mailbox ADD COLUMN delimiter TEXT;
