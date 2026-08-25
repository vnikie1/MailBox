# Architecture

---

## 1. Stack decision

| Option                        | Pixel fidelity to macOS                              | Perf / RAM                                                   | Mail-protocol maturity                                         | Verdict                         |
| ----------------------------- | ---------------------------------------------------- | ------------------------------------------------------------ | -------------------------------------------------------------- | ------------------------------- |
| **Tauri 2 + React/TS + Rust** | Excellent — full CSS control                         | Excellent (~80–150MB, ~10MB installer, uses system WebView2) | Good — `async-imap`, `mail-parser`, `lettre`                   | **Recommended**                 |
| Electron + React/TS + Node    | Excellent                                            | Poor (~300–500MB, ~120MB installer)                          | Excellent — `imapflow`, `mailparser`, `nodemailer`             | Fallback if the team is JS-only |
| WinUI 3 + C#/.NET             | Hard — you fight the control templates for months    | Excellent                                                    | Excellent — `MailKit` is the best mail library on any platform | Only if the team is .NET-native |
| Avalonia + C#                 | Good — full control, but you rebuild every primitive | Good                                                         | Excellent — `MailKit`                                          | Viable dark horse               |

**Decision: Tauri 2 (Rust core) + React 19 + TypeScript + Vite.**

Reasoning: the entire value of this project is _visual and interaction fidelity_, and CSS is
the only toolkit that gets you there in reasonable time. Tauri keeps the footprint honest and
puts the protocol work in Rust where the concurrency, TLS and parsing story is strong. WebView2
ships with Windows 11, so there is no bundled runtime.

**Escape hatch:** if Rust IMAP maturity becomes the bottleneck, swap the sync core to a
sidecar .NET process using **MailKit** and keep the Tauri UI. The IPC contract in §4 is
designed so the core is replaceable.

### Pinned dependencies

**Rust core**

```
tauri 2            app shell, IPC, windowing, updater
tokio              async runtime
async-imap         IMAP client (IDLE, CONDSTORE)
async-native-tls   TLS
mail-parser        MIME parsing (Stalwart) — handles real-world broken MIME
mail-builder       MIME construction
lettre             SMTP submission
rusqlite (bundled, FTS5)  local store
r2d2               connection pool
keyring            Windows Credential Manager
oauth2             OAuth 2.0 PKCE
ammonia            HTML sanitisation
reqwest            OAuth token exchange, remote-content proxy
tracing            structured logging
```

**Frontend**

```
react 19 + react-dom
typescript 5
vite 6
zustand              UI state (small, no boilerplate)
@tanstack/react-query  server-state / IPC cache
@tanstack/react-virtual  list virtualisation (mandatory)
@floating-ui/react   popovers, menus, tooltips
lucide-react         icons
@lexical/react       compose rich-text editor
date-fns             relative date formatting
vitest + @testing-library/react   unit
@playwright/test     e2e against the built app
```

Deliberately **not** used: Tailwind (fights a token system this specific), any component
library (MUI/Radix themes/shadcn — all bring a non-Apple visual language you would spend
longer removing than writing from scratch). Radix **primitives** (unstyled) are acceptable
if the team prefers them to Floating UI.

---

## 2. Process and thread model

```
┌─ Tauri main process (Rust) ────────────────────────────────────────┐
│  window manager · tray · toasts · jump list · protocol handler     │
│  ┌ tokio runtime ─────────────────────────────────────────────┐    │
│  │  SyncSupervisor                                             │    │
│  │    └─ AccountWorker  (one task per account)                 │    │
│  │         ├─ IdleConnection      (long-lived, 1 per account)  │    │
│  │         ├─ FetchPool           (2–4 conns, backfill/bodies) │    │
│  │         └─ SubmitQueue         (SMTP, retrying)             │    │
│  │  DbActor  (single writer, serialised; readers use a pool)   │    │
│  │  OutboxScheduler (undo-send timers, send-later, snooze)     │    │
│  └─────────────────────────────────────────────────────────────┘    │
├─ WebView2 (main window) ──────────┬─ WebView2 (compose window N) ───┤
│  React UI                          │  React UI (compose route)      │
└────────────────────────────────────┴────────────────────────────────┘
```

**Rules:**

- The UI **never** talks to a mail server. It reads and writes the local DB via IPC only.
- SQLite: one writer actor, many readers via a pool. WAL mode.
- Every compose window is a real OS window (matching Mail), sharing the same Vite bundle at a
  different route.

---

## 3. Data model (SQLite, WAL, FTS5)

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous  = NORMAL;
PRAGMA foreign_keys = ON;

CREATE TABLE account (
  id            INTEGER PRIMARY KEY,
  display_name  TEXT NOT NULL,
  email         TEXT NOT NULL UNIQUE,
  provider      TEXT NOT NULL,            -- gmail | outlook | icloud | imap
  imap_host TEXT, imap_port INTEGER, imap_security TEXT,
  smtp_host TEXT, smtp_port INTEGER, smtp_security TEXT,
  auth_kind     TEXT NOT NULL,            -- oauth2 | password
  cred_ref      TEXT NOT NULL,            -- Credential Manager key; NEVER a secret
  color         TEXT,
  sort_order    INTEGER DEFAULT 0,
  sync_enabled  INTEGER DEFAULT 1
);

CREATE TABLE mailbox (
  id           INTEGER PRIMARY KEY,
  account_id   INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  remote_path  TEXT NOT NULL,             -- IMAP path, raw
  display_name TEXT NOT NULL,
  parent_id    INTEGER REFERENCES mailbox(id),
  role         TEXT,                      -- inbox|drafts|sent|junk|trash|archive|all|null
  uid_validity INTEGER,
  uid_next     INTEGER,
  highest_modseq INTEGER,                 -- CONDSTORE
  unread_count INTEGER DEFAULT 0,
  total_count  INTEGER DEFAULT 0,
  subscribed   INTEGER DEFAULT 1,
  UNIQUE(account_id, remote_path)
);

CREATE TABLE message (
  id            INTEGER PRIMARY KEY,
  account_id    INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  mailbox_id    INTEGER NOT NULL REFERENCES mailbox(id) ON DELETE CASCADE,
  uid           INTEGER NOT NULL,
  message_id    TEXT,                     -- RFC Message-ID header
  thread_id     INTEGER REFERENCES thread(id),
  in_reply_to   TEXT,
  references_   TEXT,                     -- space-joined
  subject       TEXT,
  subject_base  TEXT,                     -- Re:/Fwd: stripped, for threading
  from_name TEXT, from_addr TEXT,
  to_json TEXT, cc_json TEXT, bcc_json TEXT, reply_to_json TEXT,
  date_sent     INTEGER NOT NULL,         -- epoch seconds, UTC
  date_received INTEGER NOT NULL,
  size          INTEGER,
  preview       TEXT,                     -- cleaned, ~300 chars
  flag_seen INTEGER DEFAULT 0,
  flag_answered INTEGER DEFAULT 0,
  flag_flagged  INTEGER DEFAULT 0,
  flag_draft    INTEGER DEFAULT 0,
  flag_deleted  INTEGER DEFAULT 0,
  flag_color    TEXT,                     -- red|orange|...|gray
  has_attachment INTEGER DEFAULT 0,
  is_junk       INTEGER DEFAULT 0,
  snooze_until  INTEGER,
  body_state    TEXT DEFAULT 'none',      -- none|headers|full
  raw_path      TEXT,                     -- .eml on disk when body_state='full'
  body_html     TEXT,
  body_text     TEXT,
  UNIQUE(mailbox_id, uid)
);
CREATE INDEX ix_msg_list    ON message(mailbox_id, date_received DESC);
CREATE INDEX ix_msg_thread  ON message(thread_id, date_sent);
CREATE INDEX ix_msg_msgid   ON message(message_id);
CREATE INDEX ix_msg_unread  ON message(mailbox_id, flag_seen) WHERE flag_seen = 0;
CREATE INDEX ix_msg_snooze  ON message(snooze_until) WHERE snooze_until IS NOT NULL;

CREATE TABLE thread (
  id            INTEGER PRIMARY KEY,
  account_id    INTEGER NOT NULL,
  subject_base  TEXT,
  last_date     INTEGER,
  message_count INTEGER DEFAULT 1,
  unread_count  INTEGER DEFAULT 0,
  muted         INTEGER DEFAULT 0
);

CREATE TABLE attachment (
  id INTEGER PRIMARY KEY,
  message_id INTEGER NOT NULL REFERENCES message(id) ON DELETE CASCADE,
  part_id TEXT, filename TEXT, mime TEXT, size INTEGER,
  content_id TEXT, is_inline INTEGER DEFAULT 0,
  cache_path TEXT
);

CREATE TABLE contact (
  id INTEGER PRIMARY KEY,
  addr TEXT NOT NULL UNIQUE, name TEXT,
  avatar_path TEXT, is_vip INTEGER DEFAULT 0, is_blocked INTEGER DEFAULT 0,
  seen_count INTEGER DEFAULT 0, last_seen INTEGER
);

CREATE TABLE smart_mailbox (
  id INTEGER PRIMARY KEY, name TEXT NOT NULL, icon TEXT,
  match_all INTEGER DEFAULT 1, predicate_json TEXT NOT NULL, sort_order INTEGER
);

CREATE TABLE rule (
  id INTEGER PRIMARY KEY, name TEXT NOT NULL, enabled INTEGER DEFAULT 1,
  match_all INTEGER DEFAULT 1,
  predicate_json TEXT NOT NULL, actions_json TEXT NOT NULL, sort_order INTEGER
);

-- durable outbox: this is what makes Undo Send and offline sending correct
CREATE TABLE outbox (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL,
  eml_path TEXT NOT NULL,
  state TEXT NOT NULL,                  -- holding|queued|sending|sent|failed
  send_after INTEGER NOT NULL,          -- undo-send / send-later timestamp
  attempts INTEGER DEFAULT 0,
  last_error TEXT
);

-- every optimistic local change lands here first, then syncs
CREATE TABLE pending_op (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL,
  kind TEXT NOT NULL,                   -- flag|move|copy|delete|expunge|append
  payload_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  attempts INTEGER DEFAULT 0,
  last_error TEXT
);

CREATE VIRTUAL TABLE message_fts USING fts5(
  subject, body, from_all, to_all, attachment_names,
  content='', tokenize='unicode61 remove_diacritics 2'
);
```

**Why `pending_op` matters:** it is the mechanism behind "deleting is 0ms". The UI mutates
`message`, inserts a `pending_op`, and returns. The worker drains ops with retry and
exponential backoff. On conflict, server state wins and the UI reconciles silently.

---

## 4. IPC contract

Tauri commands, all `async`, all returning `Result<T, AppError>`. This is the seam that lets
the core be replaced.

```
accounts_list() -> Account[]
account_add(draft: AccountDraft) -> Account          // runs autodiscovery + login test
account_test(draft) -> ConnectionReport
account_remove(id, purge_local: bool)

mailboxes_tree(account_id?) -> MailboxNode[]

messages_page(query: ListQuery) -> Page<MessageRow>  // keyset paginated, never OFFSET
message_get(id) -> MessageFull                       // fetches body on demand
thread_get(thread_id) -> MessageFull[]

msg_set_flags(ids[], flags: FlagPatch)               // optimistic
msg_move(ids[], mailbox_id)
msg_delete(ids[], permanent: bool)
msg_snooze(ids[], until)

compose_new(kind: New|Reply|ReplyAll|Forward|Redirect, source_id?) -> DraftId
compose_save(draft)
compose_send(draft, delay_secs) -> OutboxId
compose_cancel_send(outbox_id) -> bool

search(query: SearchQuery) -> SearchResults          // tokens + free text
search_suggest(prefix) -> Suggestion[]

smartbox_*  rule_*  contact_*  settings_*
attachment_open(id) / attachment_save_as(id, path) / attachment_drag(id)
remote_content_allow(message_id | sender_addr)
```

**Events pushed core → UI** (Tauri event bus):

```
sync:progress   { account_id, phase, done, total }
mailbox:changed { mailbox_id, unread, total }
messages:added  { mailbox_id, ids[] }
messages:updated{ ids[] }
messages:removed{ ids[] }
outbox:changed  { id, state }
account:error   { account_id, code, message, retry_at }
```

The UI subscribes and invalidates React Query keys. **No polling from the UI, ever.**

---

## 5. Sync engine

### Connection strategy per account

- **1 IDLE connection** parked on the Inbox. Re-issue IDLE every 25 minutes (RFC 2177 / NAT).
- **2–4 pooled connections** for backfill, body fetch and flag writes.
- Reconnect with jittered exponential backoff: 1s → 2 → 4 → … → 300s cap.
- Detect capability once per connection: `CONDSTORE`, `QRESYNC`, `MOVE`, `IDLE`,
  `X-GM-EXT-1`, `SPECIAL-USE`, `COMPRESS=DEFLATE`, `OBJECTID`.

### Initial sync (target: usable in under 10 seconds)

1. `LIST`/`LSUB` → build the mailbox tree; infer roles from `SPECIAL-USE`, falling back to
   name heuristics per provider.
2. For the Inbox: fetch **envelopes only** for the newest 500 UIDs. Render immediately.
3. Backfill older envelopes in batches of 500, lowest priority, pausing on user interaction.
4. Fetch bodies **lazily** on selection, plus a prefetch of the next 3 rows in the list.
5. Other mailboxes: envelopes for the newest 200 each, on first visit.

### Incremental sync

- **With CONDSTORE:** `SELECT` returns `HIGHESTMODSEQ`; then
  `UID FETCH 1:* (FLAGS) (CHANGEDSINCE <stored_modseq>)` plus `VANISHED` if QRESYNC.
- **Without:** compare `UIDNEXT` for additions and run a windowed `UID FETCH (FLAGS)` over the
  most recent 2,000 UIDs for changes.
- On `UIDVALIDITY` change: drop and re-sync that mailbox. Do not try to be clever.
- Gmail: use `X-GM-THRID` for threading and `X-GM-LABELS` for labels-as-mailboxes; treat
  `[Gmail]/All Mail` as the archive and never double-count messages that appear in multiple
  labels (key on `X-GM-MSGID`).

### Threading (non-Gmail)

Implement the **JWZ algorithm**:

1. Build an id-table from `Message-ID`, `In-Reply-To`, `References`.
2. Link children to parents; break cycles.
3. Group root sets by `subject_base` (strip `Re:`, `Fwd:`, `AW:`, `RE :`, `[list]` prefixes,
   case- and whitespace-insensitive) only where no reference link exists.
4. Persist `thread_id`; re-thread incrementally when a message arrives that bridges two threads
   (merge, lower id wins).

### Sending

1. Build MIME with `mail-builder`: `text/plain` + `text/html` alternative, inline images as
   `multipart/related` with `cid:`, attachments as `multipart/mixed`.
2. Write the `.eml` to disk and insert into `outbox` with `state='holding'`,
   `send_after = now + undo_delay`.
3. UI shows the Undo banner. Cancel = delete the row. No network happened.
4. At `send_after`, transition to `queued`, SMTP-submit, then `APPEND` to the Sent mailbox
   (skip the APPEND for Gmail, which does it server-side).
5. Failures: retry 3x with backoff, then `state='failed'` and a persistent banner with Retry
   and Edit. **Never silently drop a message.**

### Performance budgets (enforce in CI)

| Metric                                     | Budget                                            |
| ------------------------------------------ | ------------------------------------------------- |
| Cold start to painted UI                   | < 800 ms                                          |
| Mailbox switch (10k messages)              | < 80 ms                                           |
| Message select → body painted (cached)     | < 50 ms                                           |
| Search keystroke → results (100k messages) | < 120 ms                                          |
| Scroll                                     | 60 fps sustained, no dropped frames over 100 rows |
| Idle RAM, 3 accounts / 100k messages       | < 300 MB                                          |
| Idle CPU                                   | < 0.5 %                                           |

---

## 6. Rendering mail safely

Message bodies are **hostile input**. Non-negotiable:

1. Render in a **sandboxed `<iframe sandbox="allow-same-origin">`** — no `allow-scripts`,
   no `allow-popups`, no `allow-top-navigation`.
2. Sanitise server-side in Rust with `ammonia` before it ever reaches the WebView: strip
   `<script>`, `<iframe>`, `<object>`, `<embed>`, `<form>`, all `on*` attributes,
   `javascript:` / `data:` (except `data:image/*`) URLs, and `<meta http-equiv="refresh">`.
3. **Block remote content by default.** Rewrite `src` to a `blocked:` placeholder and show the
   "Load Remote Content" banner. On load, proxy through the Rust core so the sender never sees
   the user's IP, and strip the `Referer`.
4. Inline `cid:` images resolve from the local attachment cache only.
5. CSP on the frame: `default-src 'none'; img-src cid: app: data:; style-src 'unsafe-inline';`
6. Links open in the **default browser**, never in the WebView. Show a confirmation when the
   visible link text is a URL that differs in host from the `href` (phishing check).
7. Auto-size the iframe by posting `document.documentElement.scrollHeight` on load and via a
   `ResizeObserver`; clamp wide tables with `overflow-x: auto` rather than letting them force
   horizontal page scroll.

---

## 7. Credentials and auth

- **Never** store a secret in SQLite, `localStorage`, or a config file. Only a `cred_ref` key.
- Secrets live in the **Windows Credential Manager** via the `keyring` crate; DPAPI encrypts
  them per-user at rest.
- **OAuth 2.0 with PKCE, loopback redirect** (`http://127.0.0.1:<random>/callback`), opened in
  the **system browser**, not an embedded WebView. Requires:
  - **Gmail** — Google Cloud project, `https://mail.google.com/` scope, OAuth consent screen.
    Restricted scopes require verification + a CASA security assessment before you can go past
    100 test users. See `06-risks-and-legal.md`.
  - **Microsoft/Outlook** — Entra app registration, `IMAP.AccessAsUser.All`,
    `SMTP.Send`, `offline_access`.
  - **iCloud** — no OAuth for third parties; requires an **app-specific password**. Ship a
    guided flow that links to appleid.apple.com and explains it.
- Refresh tokens proactively 5 minutes before expiry; on `invalid_grant`, surface a
  re-authenticate banner rather than a silent failure.
- Support plain IMAP/SMTP with `AUTH PLAIN`/`LOGIN` over TLS, plus manual host/port entry and
  autodiscovery via the Mozilla ISPDB + `autoconfig.<domain>` + SRV records.

---

## 8. Windows integration checklist

| Feature                                                  | How                                                                           |
| -------------------------------------------------------- | ----------------------------------------------------------------------------- |
| Toast notifications with Reply/Archive/Mark Read actions | Windows AppNotification + COM activator; requires an AUMID from the installer |
| Taskbar unread badge                                     | `ITaskbarList3::SetOverlayIcon`                                               |
| Jump List: New Message, Inbox, Search                    | `ICustomDestinationList`                                                      |
| `mailto:` default handler                                | Register capability in the installer; handle the argv on launch               |
| Tray icon with unread count + quick actions              | Tauri tray API                                                                |
| Run at login                                             | `HKCU\...\Run` or an MSIX startup task, user-toggleable                       |
| System theme + accent following                          | `UISettings.ColorValuesChanged` → push to the WebView                         |
| Acrylic / Mica backdrop                                  | `DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE)`                            |
| Snap Layouts                                             | Works if the custom titlebar leaves a real maximize button hit-region         |
| Per-monitor DPI v2                                       | Manifest declaration; verify at 100/125/150/175 %                             |
| Drag attachment to Explorer                              | `DoDragDrop` with `CFSTR_FILEDESCRIPTOR` / `CFSTR_FILECONTENTS`               |
| File association for `.eml`                              | Optional; opens a read-only viewer window                                     |
| Windows Search indexing                                  | Optional IFilter; defer past v1                                               |

---

## 9. Repository layout

```
mac-mail-win/
├─ src-tauri/
│  ├─ src/
│  │  ├─ main.rs
│  │  ├─ ipc/            command handlers, one module per domain
│  │  ├─ db/             schema, migrations, queries, fts
│  │  ├─ sync/           supervisor, account_worker, idle, backfill, threading
│  │  ├─ mime/           parse, build, preview extraction, sanitise
│  │  ├─ smtp/           submit, outbox scheduler
│  │  ├─ auth/           oauth, keyring, autodiscover
│  │  ├─ rules/          predicate engine (shared by rules + smart mailboxes)
│  │  └─ platform/       toasts, jumplist, tray, backdrop, dragdrop
│  ├─ migrations/
│  └─ tauri.conf.json
├─ src/
│  ├─ app/               routes: main, compose, settings, viewer
│  ├─ features/
│  │  ├─ sidebar/  message-list/  reader/  compose/  search/
│  │  ├─ settings/ onboarding/    rules/   smart-mailboxes/
│  ├─ ui/                primitives: Button, Menu, Popover, Field, Chip, Icon…
│  ├─ styles/            tokens/primitive.css, semantic.css, component.css
│  ├─ lib/               ipc client, query keys, formatters, shortcuts
│  └─ store/             zustand slices
├─ docs/                 these documents
├─ assets/reference/     macOS screenshots for pixel comparison
└─ tests/                vitest unit, playwright e2e, rust integration
```

---

## 10. Testing strategy

| Layer            | Tool                                                            | What                                                                   |
| ---------------- | --------------------------------------------------------------- | ---------------------------------------------------------------------- |
| MIME parsing     | Rust unit + a corpus of real-world broken emails                | encodings, nested multipart, malformed headers, RTL, huge attachments  |
| Threading        | Rust unit, fixture mailboxes                                    | JWZ correctness, merges, subject-only grouping, Gmail THRID            |
| Sync             | Rust integration against **GreenMail** or **Dovecot in Docker** | initial sync, CONDSTORE deltas, UIDVALIDITY reset, offline queue drain |
| Predicate engine | Rust property tests                                             | rules and smart mailboxes share one implementation                     |
| Sanitiser        | Rust, with an XSS payload corpus                                | must block every payload; snapshot the output                          |
| UI components    | Vitest + Testing Library                                        | states, keyboard, a11y roles                                           |
| Visual           | Playwright screenshots vs `assets/reference/`                   | per-component diff at a 2px threshold                                  |
| E2E              | Playwright driving the built app                                | add account → read → reply → send → search → undo                      |
| Perf             | Playwright traces + a seeded 100k-message DB                    | assert the §5 budgets in CI                                            |

---

## 11. Build, packaging, distribution

- **Installer:** NSIS via Tauri for the free/self-distributed path; **MSIX** additionally if
  you want Store distribution or clean AUMID registration for toasts.
- **Code signing:** required, or SmartScreen will scare every user. An OV cert now needs an
  eToken or a cloud HSM (Azure Trusted Signing is the cheapest sane route).
- **Auto-update:** Tauri updater with a signed `latest.json` manifest, delta not required.
- **Crash reporting:** local-only crash dumps by default; opt-in upload. No telemetry by
  default — it is part of the product promise.
- **Data location:** `%LOCALAPPDATA%\<AppName>\` for the DB, attachment cache, and `.eml`
  store. Provide Export (mbox / .eml tree) and a full-wipe action in Settings.
