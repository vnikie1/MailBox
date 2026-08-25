# Roadmap — 12 phases, with exit criteria

Each phase has a **demo** (what you can show), an **exit gate** (objective, testable) and a
rough size. Do not start a phase until the previous gate passes. Effort assumes one competent
full-time developer working with an AI coding agent.

---

## Phase 0 — Foundation (2–3 days)

Scaffold Tauri 2 + React 19 + TS + Vite. ESLint, Prettier, `rustfmt`, `clippy`, Vitest,
Playwright, GitHub Actions CI. Custom titlebar with Windows caption buttons. Acrylic backdrop.
Light/dark theming wired to the OS. Token files in place, empty.

**Exit gate:** app launches, window drags/resizes/snaps correctly, theme follows Windows
instantly, caption buttons behave exactly like native ones (including Snap Layouts hover),
CI green on a clean clone.

---

## Phase 1 — Design system (3–5 days)

Implement `02-design-system.md` in full: all three token files, plus the primitives —
`Button`, `IconButton`, `Menu`, `ContextMenu`, `Popover`, `Tooltip`, `Field`, `TokenField`,
`Chip`, `Avatar`, `Badge`, `Divider`, `Sheet`, `Toast`, `Skeleton`.

Build a `/dev/gallery` route showing every primitive in every state, in both themes.

**Exit gate:** gallery renders all primitives; zero hardcoded colours/sizes anywhere
(enforce with a stylelint rule); visual diff of the gallery is stable; keyboard nav works on
every interactive primitive.

---

## Phase 2 — Static shell with mock data (1 week)

The whole three-pane UI, pixel-targeted, driven by a fixture JSON of ~2,000 fake messages.
Sidebar, virtualised message list with all row states, reader with a mock body, toolbar,
search field (visual only), responsive collapse at 1000px and 700px, sidebar collapse,
classic-layout toggle, density modes.

**This is the phase that decides whether the project succeeds.** Do not rush it, and do not
proceed until it genuinely looks like Mail.

**Exit gate:** side-by-side screenshots against `assets/reference/` at matched widths, in both
themes, reviewed and signed off. Scrolling 2,000 rows holds 60 fps. Every item on the
§8 Visual QA checklist in `02-design-system.md` passes.

---

## Phase 3 — Local data layer (4–5 days)

SQLite schema and migrations, DbActor, connection pool, FTS5 triggers, keyset pagination,
a seed tool that generates 100k realistic messages. Swap the UI from fixtures to real IPC.

**Exit gate:** 100k-message DB; mailbox switch < 80 ms; list scroll still 60 fps; FTS query
< 120 ms; migrations run forward cleanly on a fresh and an existing DB.

---

## Phase 4 — Accounts and auth (1 week)

Onboarding flow modelled on Mail's account assistant: provider picker (Google, Microsoft,
iCloud, Yahoo, Other), OAuth PKCE via system browser, app-specific-password guidance for
iCloud, manual IMAP/SMTP entry with autodiscovery, connection test with a clear diagnostic
report, Credential Manager storage, multi-account, account removal with local purge.

**Exit gate:** real Gmail, Outlook, iCloud and a generic IMAP host all connect; tokens refresh
across an app restart; no secret appears in the DB, logs, or on disk (grep-verified).

---

## Phase 5 — Sync engine (2 weeks — the hardest phase)

Mailbox tree discovery and role inference; initial envelope sync; lazy body fetch with
prefetch; IDLE; CONDSTORE/QRESYNC deltas; `UIDVALIDITY` recovery; backfill with
backpressure; JWZ threading + Gmail `X-GM-THRID`; `pending_op` drain with retry;
offline mode; reconnect with jittered backoff; per-account error surfacing.

**Exit gate:** against Dovecot-in-Docker _and_ a real Gmail account —
cold sync of a 50k-message mailbox completes and stays correct; killing the network mid-sync
recovers without duplicates or loss; flags changed on another device appear within 5 s;
`UIDVALIDITY` reset is handled; a 12-hour soak shows no leak and no connection storm.

---

## Phase 6 — Reading (1 week)

Sandboxed iframe renderer, Rust-side sanitiser, remote-content blocking + proxy, `cid:` inline
images, plain-text rendering, thread stack view with collapse/expand, quoted-text collapsing,
attachment chips, built-in previewer (image/PDF/text), save and drag-to-Explorer, contact
popover, data detectors (date → calendar `.ics`, address, phone, tracking numbers), reader
banners.

**Exit gate:** the XSS payload corpus is fully blocked; no message triggers a network request
before consent; 20 real newsletters from major senders render correctly in both themes; PDF
and image previews work; drag-to-Explorer produces a valid file.

---

## Phase 7 — Composing and sending (1.5 weeks)

Compose as a separate window; Lexical rich-text editor with the Mail formatting bar; recipient
token fields with contact autocomplete; From/alias picker; signatures (per-account, rich, with
placement options); attachments with size warnings; drafts with autosave and cross-device
sync via IMAP `APPEND`; reply/reply-all/forward/redirect quoting rules; MIME building; SMTP
submission; durable outbox; **Undo Send**; **Send Later**; failure recovery UI.

**Exit gate:** replies quote correctly and thread correctly in Gmail, Outlook and Apple Mail
on the receiving end; HTML renders correctly in Outlook Windows (the strictest renderer);
undo genuinely prevents transmission; a send queued while offline goes out on reconnect;
killing the app mid-send does not lose or duplicate the message.

---

## Phase 8 — Organisation (1 week)

Flags with 7 colours; VIPs; mark junk with a local Bayesian classifier trained on user
actions; block sender; move/copy with drag-and-drop; the shared predicate engine; Smart
Mailbox editor; Rules editor with run-on-demand; mute thread; **Remind Me** (snooze) and
**Follow Up**; full Undo stack (Ctrl+Z across delete, move, archive, flag, send).

**Exit gate:** a rule created in the UI fires on incoming mail and on manual run; a smart
mailbox with 5 predicates returns correct results against the 100k seed; undo restores exact
prior state for every action type; junk classifier beats 90 % on a labelled test corpus.

---

## Phase 9 — Search (1 week)

FTS5 index over subject/body/participants/attachment names; the token query language and its
parser; suggestion dropdown; scope bar; Top Hits ranking; result highlighting in the reader;
attachment-content indexing for PDF/DOCX/TXT; save search as Smart Mailbox; search history.

**Exit gate:** < 120 ms at 100k messages for both single-term and multi-token queries;
suggestions appear within 30 ms of a keystroke; token UX matches the spec in
`01-macos-mail-analysis.md` §7; results ranked sensibly (recency × relevance × VIP).

---

## Phase 10 — Polish and platform (1 week)

Every keyboard shortcut from §14; toast notifications with inline Reply/Archive/Mark Read;
taskbar badge; jump list; tray; `mailto:` handler; run-at-login; sounds; swipe gestures;
all animations tuned to the motion tokens; empty and error states for every surface;
full accessibility pass (screen reader labels, focus order, contrast, reduced motion,
reduce transparency, 200 % text scaling).

**Exit gate:** a full triage session completed without touching the mouse; Narrator can read
and act on the message list; every animation matches its token; no unstyled or dead-end state
reachable in the app.

---

## Phase 11 — Ship (1 week)

Settings window (General, Accounts, Composing, Signatures, Rules, Privacy, Advanced);
import from Thunderbird/Outlook/mbox; export; onboarding first-run experience; app icon set;
NSIS + MSIX packaging; code signing; auto-updater; crash handling; privacy policy;
README and user docs.

**Exit gate:** clean install on a fresh Windows 11 VM, no SmartScreen warning, first-run to
reading mail in under 3 minutes, update from the previous version preserves all data,
uninstall leaves nothing behind unless the user opts to keep data.

---

## Total: roughly 11–13 weeks for v1

**Critical path:** Phase 2 (does it look right?) and Phase 5 (does sync actually work?).
Everything else is tractable. If time is short, cut Phases 8 and 9 features — not Phase 2.

---

## Post-v1 backlog

| Priority | Item                                                                              |
| -------- | --------------------------------------------------------------------------------- |
| High     | Mail categories (Primary/Transactions/Updates/Promotions) with a local classifier |
| High     | JMAP support (Fastmail) — dramatically better than IMAP where available           |
| High     | Exchange Web Services / Microsoft Graph for corporate accounts without IMAP       |
| Medium   | Unified inbox across accounts                                                     |
| Medium   | PGP and S/MIME sign/encrypt                                                       |
| Medium   | Templates and scheduled follow-up reminders                                       |
| Medium   | Calendar/invite handling (accept/decline inline)                                  |
| Low      | Liquid Glass theme variant                                                        |
| Low      | Local LLM summarisation and smart reply (fully offline)                           |
| Low      | Plugin API                                                                        |
