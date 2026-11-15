-- The full-text index is updated only when the text it indexes changes. M19, 2026-11-14 audit.
--
-- `message_fts_update` fired on every UPDATE of a message, whatever it changed, and re-indexed
-- the whole message each time: its subject, its addresses and its entire body, removed and
-- written again. Marking a message read did that. So did flagging one, moving one, and every row
-- the threading pass touched, which is up to the whole account at the end of a cold sync or an
-- import. Nothing in the index had changed in any of them.
--
-- Now it fires only when a statement sets one of the five indexed columns, and only does the work
-- when one of them actually differs. The external-content index stays exact: its 'delete' needs
-- the values it indexed, and when none of them changed those are still the old row's.

DROP TRIGGER message_fts_update;

CREATE TRIGGER message_fts_update
AFTER UPDATE OF subject, body_text, from_all, to_all, attachment_names ON message
WHEN old.subject IS NOT new.subject
  OR old.body_text IS NOT new.body_text
  OR old.from_all IS NOT new.from_all
  OR old.to_all IS NOT new.to_all
  OR old.attachment_names IS NOT new.attachment_names
BEGIN
  INSERT INTO message_fts(message_fts, rowid, subject, body_text, from_all, to_all, attachment_names)
  VALUES ('delete', old.id, old.subject, old.body_text, old.from_all, old.to_all, old.attachment_names);
  INSERT INTO message_fts(rowid, subject, body_text, from_all, to_all, attachment_names)
  VALUES (new.id, new.subject, new.body_text, new.from_all, new.to_all, new.attachment_names);
END;
