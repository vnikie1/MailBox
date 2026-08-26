macOS Mail reference captures
=============================

Captured 2026-08-25 from a Mac Studio on the LAN.

  macOS          26.6.2 (build 25G83)
  Display        4096 x 2560 physical, "UI looks like" 2048 x 1280
  Scale          2x Retina  ->  1 logical point = 2 pixels in these files
  Mail window    2664 x 2320 px  =  1332 x 1160 pt

IMPORTANT: halve every pixel measurement taken from these files before comparing it
against docs/01 and docs/02, which are written in logical points (~ CSS px).

Files
-----
  mail-window-light-active.png     light theme, window focused      <- the primary reference
  mail-window-dark-active.png      dark theme, window focused       <- the primary reference
  mail-window-light-inactive.png   light theme, window NOT focused
  mail-window-dark-inactive.png    dark theme, window NOT focused

The inactive pair is the reference for standing rule "the window goes quiet when inactive"
(docs/01 §9.11): traffic lights grey out, the sidebar selection loses its tint, and the
whole window desaturates.

How these were taken
--------------------
macOS blocks `screencapture` run over SSH (TCC), so they were captured from Terminal on
the Mac itself and pulled across with scp. Cropping to the window was done on the Mac with
`sips -c 2320 2664 --cropOffset 64 16`. To recapture, run on the Mac:

    screencapture -T 10 -x ~/mailref/shot.png

then click the Mail window and wait ten seconds, so Mail is the active window in the shot.

Known departures from docs/01 and docs/02
-----------------------------------------
The specs were written against an older macOS. This machine is on macOS 26, and Mail has
changed. Recorded here so nothing gets "fixed" back to the spec by mistake:

  1. There is no single unified 52pt toolbar spanning the window. Each pane carries its
     own header: sidebar (traffic lights + sidebar toggle), message list (mailbox name,
     count line, filter and overflow buttons), reading pane (the action toolbar).
  2. Toolbar button order differs from docs/02 §6.1. Actual left-to-right in the reading
     pane header: compose | reply, reply-all, forward | archive, delete, junk | move |
     flag, flag-menu | search.
  3. The message list has a category filter row above it (person / cart / chat /
     megaphone / All Mail) that the specs do not mention.
  4. A "Summarise" button (Apple Intelligence) sits at the top right of the reading pane.
  5. Preview lines are AI-generated summaries prefixed with a small glyph, not the raw
     first line of the message body.

Preliminary measurements (eyeballed, to be confirmed in Phase 2)
---------------------------------------------------------------
  message list row, 2-line preview   ~80 pt   (docs/02 §6.3 says 78 - close)
  sidebar row                        ~32 pt   (docs/02 §6.2 says 28 - differs)
  sidebar width                      ~188 pt  (docs/01 §2 says 232 - but user-resizable,
                                               so this is not a fidelity signal)
