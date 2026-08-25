# Prompt library — one prompt per phase

Paste these into the session **in order**, after `PROMPT.md` has been established as context.
Each assumes the previous phase's exit gate has passed.

Each prompt follows the same shape: **Goal → Build → Constraints → Exit gate**. Keep that shape
if you write your own.

---

## Phase 1 — Design system

```
Phase 1: implement the design system from docs/02-design-system.md.

Build:
- src/styles/tokens/primitive.css, semantic.css, component.css — every token in the doc,
  nothing extra, nothing missing.
- Theme switching driven by [data-theme] on <html>, wired to the Windows theme, plus a manual
  override in a settings store. Also implement [data-window-inactive] desaturation as a single
  token remap.
- Primitives in src/ui/, each a CSS Module: Button (filled/bordered/plain/destructive),
  IconButton, Menu, ContextMenu, Popover, Tooltip, TextField, TokenField, Chip, Avatar,
  Badge, Divider, Sheet, Toast, Skeleton, ScrollArea.
- A /dev/gallery route rendering every primitive in every state, in both themes, with a
  theme toggle and a density toggle.

Constraints:
- Add a stylelint rule that fails the build on any hardcoded hex, px, ms or cubic-bezier value
  in a component CSS Module. Only the token files may contain raw values.
- Every interactive primitive: keyboard operable, :focus-visible ring, correct ARIA role,
  works with prefers-reduced-motion.
- Floating layers use Floating UI with proper collision handling and a safe-triangle submenu.

Exit gate: gallery shows all primitives in all states in both themes; stylelint passes;
Vitest covers keyboard behaviour for Menu, TokenField and Popover; a Playwright screenshot of
the gallery is committed as the visual baseline.
```

---

## Phase 2 — Static shell (the make-or-break phase)

```
Phase 2: build the complete three-pane UI against mock data. No networking, no database.

Build:
- A fixture generator producing ~2,000 realistic messages: varied senders, subjects, threads
  of 1-15 messages, attachments, flags, read/unread, dates spread over 2 years.
- Titlebar/toolbar exactly per docs/02 §6.1, including the Windows caption buttons.
- Sidebar per docs/01 §3 and docs/02 §6.2: Favourites, Smart Mailboxes, per-account trees,
  disclosure animation, unread badges, no hover highlight, drag-drop target styling.
- Message list per docs/01 §4 and docs/02 §6.3: TanStack Virtual, all row states, sticky date
  section headers, thread count pills, sort menu, preview-line setting 0-5, contact photos
  with deterministic initials fallback, multi-select with contiguous-run merging.
- Reader per docs/01 §5 and docs/02 §6.4: header, recipient expansion, hover action glyphs,
  banners, mock body, attachment chips, thread stack with collapse/expand.
- Responsive collapse: 3 panes -> 2 at 1000px -> 1 with push navigation at 700px. Sidebar
  collapse (Ctrl+Shift+S). Classic-layout toggle. Density modes.
- Draggable pane dividers with persisted widths and min/max clamps.

Constraints:
- Everything from docs/02 §8 Visual QA checklist must pass before you call this done.
- 60 fps scrolling over 2,000 rows; measure it and show me the trace.
- Nothing shifts when read/flag state changes — reserve the gutters.

Exit gate: screenshots of each pane in both themes at 1400px and 900px window width, placed
next to the macOS references in assets/reference/, with a written comparison of every
discrepancy you can see and whether you fixed it or why you couldn't.
```

> **Note:** before starting Phase 2, put real macOS Mail screenshots in `assets/reference/`.
> Capture at least: light and dark, three-pane and two-pane, sidebar expanded and collapsed,
> a thread with 5+ messages, a compose window, a search-with-tokens state, and a message with
> attachments. Without these, "pixel-perfect" is unverifiable.

---

## Phase 3 — Data layer

```
Phase 3: implement the local store from docs/03-architecture.md §3, and switch the UI off
fixtures onto real IPC.

Build:
- SQLite schema + a forward-only migration runner. WAL, foreign keys on.
- DbActor: single writer task, serialised; reader pool via r2d2. All writes in transactions.
- FTS5 external-content table with triggers keeping it in sync with `message`.
- Keyset-paginated messages_page() — never OFFSET. Cursor on (date_received, id).
- The Tauri command surface from docs/03 §4, with typed TS bindings generated from the Rust
  types (specta/ts-rs) so the contract cannot drift.
- The event bus: sync:progress, mailbox:changed, messages:added/updated/removed, outbox:changed,
  account:error. UI subscribes and invalidates TanStack Query keys.
- A `seed` dev binary generating 100,000 realistic messages across 3 accounts and 40 mailboxes.

Constraints:
- No SQL string interpolation anywhere. Parameterised only.
- Every query used by the list must have a supporting index; show me EXPLAIN QUERY PLAN output
  for messages_page, the unread count, and the FTS query.

Exit gate: against the 100k seed — mailbox switch < 80ms, list scroll still 60fps, FTS query
< 120ms, cold start < 800ms. Show the measurements.
```

---

## Phase 4 — Accounts and auth

```
Phase 4: account setup and credential handling. Read docs/05-risks-and-legal.md first.

Build:
- Onboarding flow modelled on Mail's account assistant: provider picker (Google, Microsoft,
  iCloud, Yahoo, Other), then provider-specific paths.
- OAuth 2.0 + PKCE with a loopback redirect on a random port, opened in the SYSTEM BROWSER.
  Never an embedded WebView — Google blocks that.
- Guided app-specific-password flow for iCloud with the exact steps and a deep link.
- Manual IMAP/SMTP entry with autodiscovery: Mozilla ISPDB, then autoconfig.<domain>, then
  SRV records, then port probing. Show what it found and let the user override.
- Connection test producing a readable diagnostic report — which step failed, what the server
  said, what the user should do. Not "authentication failed".
- Credential storage via the keyring crate. Token refresh 5 minutes before expiry; on
  invalid_grant, a re-authenticate banner, not a silent failure.
- Multi-account, account reordering, per-account colour, remove-with-purge.

Constraints:
- After a successful setup, grep the DB file, all logs and all config for the password/token
  string and show me that it appears nowhere.
- Handle the specific Microsoft failure modes: tenant blocks IMAP, SMTP AUTH disabled per
  mailbox. Each needs its own error message naming what an admin must change.

Exit gate: real Gmail, Outlook, iCloud and a generic IMAP host all connect and survive an app
restart with token refresh.
```

---

## Phase 5 — Sync engine (allow the most time here)

```
Phase 5: the IMAP sync engine per docs/03-architecture.md §5.

Build, in this order, testing each against Dovecot in Docker before moving on:
1. Mailbox tree discovery; role inference from SPECIAL-USE with per-provider name fallbacks.
2. Initial envelope sync: newest 500 UIDs of the Inbox, rendered immediately; then backfill in
   500-UID batches at low priority, pausing while the user is interacting.
3. Lazy body fetch on selection + prefetch of the next 3 rows. Cache .eml on disk.
4. IDLE on a dedicated connection, re-issued every 25 minutes, with an app-level heartbeat.
5. CONDSTORE/QRESYNC incremental sync; windowed FLAGS fetch fallback without it.
6. UIDVALIDITY change detection -> drop and re-sync that mailbox.
7. JWZ threading with tests written first; Gmail X-GM-THRID path; incremental re-threading
   when a new message bridges two threads.
8. pending_op drain with exponential backoff and per-op retry limits; offline queueing.
9. Per-account error surfacing with a retry-at time, not a spinner that never ends.

Constraints:
- Connection pool size configurable per account, default 3, hard cap 5.
- Jittered backoff on reconnect: 1s -> 2 -> 4 -> ... -> 300s. Never a tight retry loop.
- Write the threading tests before the threading code.
- No unwrap() anywhere in this module.

Exit gate: cold sync of a 50k-message mailbox completes correctly; killing the network mid-sync
recovers with no duplicates and no loss; a flag changed on another device appears within 5s;
a forced UIDVALIDITY change recovers; a 12-hour soak shows flat memory and no connection storm.
Show me the soak graph.
```

---

## Phase 6 — Reading

```
Phase 6: message rendering, per docs/03-architecture.md §6.

Build:
- Rust-side sanitiser (ammonia) with the exact allowlist from the doc. Test against an XSS
  payload corpus — write that corpus first and commit it.
- Sandboxed iframe renderer, sandbox="allow-same-origin" only, with the specified CSP.
  Auto-height via ResizeObserver; wide tables get overflow-x:auto rather than scrolling the page.
- Remote content blocking with the banner, per-message and per-sender allow, and a Rust-side
  proxy that strips Referer so the sender never sees the user's IP.
- cid: inline image resolution from the local attachment cache.
- Plain-text rendering in the UI font with URL auto-linking — not monospace.
- Thread stack: all messages listed, collapsed except the newest, height-animated expansion.
- Quoted-text collapsing behind a grey chevron; detect quote blocks by >, blockquote, and the
  common "On <date>, <person> wrote:" patterns across major clients.
- Attachment chips, built-in previewer (image, PDF, text), save-as, and drag-to-Explorer via
  CFSTR_FILEDESCRIPTOR/CFSTR_FILECONTENTS.
- Contact popover; data detectors for dates (-> .ics), addresses, phone numbers, tracking numbers.
- Phishing check: warn when visible link text is a URL whose host differs from the href.

Exit gate: the XSS corpus is fully blocked (show the test output); no message causes a network
request before consent (show a network trace); 20 real newsletters from major senders render
correctly in both themes (show screenshots).
```

---

## Phase 7 — Composing and sending

```
Phase 7: compose, per docs/01-macos-mail-analysis.md §6.

Build:
- Compose as a separate OS window at the /compose route, 700x560 default, size remembered.
- Lexical editor limited to exactly Mail's format bar: bold, italic, underline, colour, size,
  alignment, bullet and numbered lists, quote, link, horizontal rule. Nothing more.
- Recipient TokenField per docs/02 §6.7: autocomplete from contacts + previous recipients,
  Enter/comma commits, chips drag between To/Cc/Bcc, invalid addresses red, duplicates deduped,
  Backspace selects the previous chip before deleting.
- From/alias picker, shown only with >1 identity.
- Signatures: per-account, rich text, placement above/below quote, per-message override.
- Attachments with a total-size warning; inline images as multipart/related with cid:.
- Drafts: autosave every 30s and on blur, IMAP APPEND to the Drafts mailbox so other devices
  see them, conflict handling if the same draft changed remotely.
- Quoting rules for reply/reply-all/forward/redirect that match Mail exactly, including the
  attribution line format and Reply-All recipient computation (honour Reply-To, exclude self,
  exclude list-unsubscribe addresses).
- MIME building: text/plain + text/html alternative, correct encoding, correct headers
  (In-Reply-To, References, Message-ID, Date, MIME-Version, User-Agent).
- Durable outbox with state machine holding -> queued -> sending -> sent/failed.
- Undo Send: 10s default (10/20/30/off), banner in the main window, cancel deletes the row
  before any network activity.
- Send Later with Tonight 9pm / Tomorrow 8am / Custom.
- Failure UI: persistent banner with Retry and Edit. Never silently drop a message.

Exit gate: send a reply to Gmail, Outlook and Apple Mail and confirm all three thread it
correctly and render the HTML correctly (Outlook Windows is the strictest — check it
specifically); a send queued while offline goes out on reconnect; killing the app mid-send
neither loses nor duplicates the message.
```

---

## Phase 8 — Organisation

```
Phase 8: flags, VIPs, junk, rules, smart mailboxes, snooze, undo.

Build:
- Flags: 7 colours with renameable labels; Flagged smart folder with per-colour children.
- VIPs (max 100) with their own mailbox and notification rules.
- Junk: local Naive Bayes classifier trained on Mark as Junk / Not Junk; junk banner in reader;
  a training-mode setting that only marks, doesn't move.
- Block sender.
- Move/copy via drag-drop to the sidebar and via a Ctrl+Shift+M mailbox picker with typeahead.
- ONE predicate engine in Rust, shared by Rules and Smart Mailboxes. Predicate fields per
  docs/01 §8. Property-test it.
- Smart Mailbox editor and Rules editor sharing the same predicate builder UI component.
  Rules add actions and run-on-demand (Alt+Ctrl+L).
- Mute thread. Remind Me (snooze) with the OutboxScheduler waking messages back to the top.
  Follow Up detection for unanswered sent mail.
- Full undo stack: Ctrl+Z across delete, move, archive, flag, mark read, and send.

Exit gate: a rule created in the UI fires on incoming mail and on manual run; a 5-predicate
smart mailbox returns correct results against the 100k seed (verify against a hand-written SQL
query); undo restores exact prior state for every action type; the junk classifier exceeds 90%
accuracy on a labelled corpus.
```

---

## Phase 9 — Search

```
Phase 9: search, per docs/01-macos-mail-analysis.md §7.

Build:
- FTS5 over subject, body, participants, attachment filenames — and attachment *contents* for
  PDF, DOCX and TXT, extracted at index time on a low-priority queue.
- Token query language + parser: from:, to:, subject:, mailbox:, has:attachment, is:unread,
  is:flagged, before:, after:, larger:. Free text combines with AND.
- Suggestion dropdown resolving a prefix into typed token candidates, grouped with headers,
  arrow-key navigable, matching docs/02 §6.6.
- Natural-language date parsing: "last week", "yesterday", "in March" -> before:/after: tokens.
- Scope bar above results (All/Inbox/Drafts/... and All/Unread/Flagged).
- Top Hits ranking: BM25 x recency decay x VIP boost x thread-participation boost. Tune it
  against real queries and show me the ranking for 5 sample searches.
- Match highlighting in the reader.
- Save search as Smart Mailbox; search history.

Exit gate: < 120ms for single-term and multi-token queries at 100k messages; suggestions within
30ms of a keystroke; show the ranked results for 5 real queries and justify the ordering.
```

---

## Phase 10 — Polish and platform

```
Phase 10: make it feel finished.

Build:
- Every keyboard shortcut in docs/01 §14, in a single central registry with a shortcuts
  reference sheet in Help.
- Toast notifications with inline Reply / Archive / Mark as Read actions (AppNotification +
  COM activator, AUMID from the installer). Per-account and VIP-only notification settings.
- Taskbar unread badge (ITaskbarList3::SetOverlayIcon), jump list (New Message, Inbox, Search),
  tray icon with unread count.
- mailto: protocol handler; .eml file association opening a read-only viewer window.
- Run at login, user-toggleable.
- Send/receive sounds, off by default.
- Swipe gestures on list rows via precision-touchpad horizontal pan and touch: rubber-banded,
  progressive colour fill, commits past 50% on release.
- Audit every animation against the motion tokens — durations, easing, and no linear stops.
- Empty, loading, offline and error states for every surface. No dead ends.
- Accessibility pass: Narrator can read and act on the list; focus order; contrast; reduced
  motion; reduce transparency; 200% text scaling.

Exit gate: complete a full triage session (read, flag, archive, reply, search, send) without
touching the mouse; Narrator walkthrough recorded; no unstyled or dead-end state reachable.
```

---

## Phase 11 — Ship

```
Phase 11: packaging and release.

Build:
- Settings window: General, Accounts, Composing, Signatures, Rules, Privacy, Advanced —
  matching Mail's structure and using the same primitives.
- Import from Thunderbird profile, Outlook PST, and mbox. Export to mbox and an .eml tree.
- First-run experience: welcome, add account, done — under 3 minutes to reading mail.
- App icon set at all required sizes; installer artwork.
- NSIS installer (and MSIX if Store distribution is wanted). Code signing — start the cert
  process now if not already done, it has weeks of lead time (docs/05 §7).
- Tauri auto-updater with a signed manifest; verify an update preserves all local data.
- Local-only crash dumps with opt-in upload. No telemetry.
- README, user docs, privacy policy, responsible-disclosure contact.

Exit gate: clean install on a fresh Windows 11 VM with no SmartScreen warning; first run to
reading mail in under 3 minutes; update from the previous version preserves everything;
uninstall leaves nothing behind unless the user opts to keep their data.
```

---

## Utility prompts (use any time)

**Fidelity audit**

```
Compare the current <component> against assets/reference/<screenshot>.png. Measure every
spacing, size, weight, opacity and radius you can determine from the reference and list every
discrepancy with the current implementation in a table: property, reference value, current
value, fixed yes/no. Then fix all of them and show a before/after screenshot pair.
```

**Performance check**

```
Run the performance budgets from docs/03-architecture.md §5 against the 100k seed database.
Report actual numbers against budget in a table. For anything over budget, profile it, tell me
where the time is going, and fix it. Do not report a budget as passing without a measurement.
```

**Security audit**

```
Audit against docs/03-architecture.md §6 and §7 and docs/05-risks-and-legal.md §6. Specifically:
grep the entire data directory and all logs for any credential material; verify the iframe
sandbox attributes and CSP at runtime; run the XSS corpus; confirm no network request fires
before remote-content consent; confirm certificate validation cannot be disabled for public
hosts. Report findings with severity, and fix everything above low.
```

**Phase gate review**

```
We are at the Phase <N> exit gate in docs/04-roadmap.md. Go through each gate criterion, state
whether it passes, and show the evidence — measurement, screenshot, or test output. Do not mark
anything as passing that you have not actually verified in the running app. List what is not
done and what it would take.
```
