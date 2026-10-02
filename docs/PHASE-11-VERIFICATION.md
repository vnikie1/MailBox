# Phase 11 — verification record

The detailed record for Phase 11 ("Ship"). `CHANGELOG.md` is the timeline; this is the evidence.

Written after the fact for the updater and uninstall gates, which are the two that had never been
run. The App Certification Kit results are in the changelog entry for 2026-08-31.

---

## 1. Updater — "an update from the previous version preserves everything"

### How it was run

The criteria were written down **before** the test, in full, including what would count as a
failure. Deciding what passes after watching something run is how a test becomes a rubber stamp.

1. `1.0.0` built and signed, with the updater endpoint pointed at `http://127.0.0.1:8787` through
   a throwaway `--config` override.
2. Installed from its own NSIS installer, per-user, no elevation.
3. `1.0.1` built and signed the same way, served by `tools/update-server.cjs`.
4. Driven through **Settings → General → Check for updates in the running app**, by UI Automation
   — not by calling the IPC command directly. The command is not the thing being tested; the path
   a user takes is.

### Result — all six criteria passed

| #   | Criterion                             | Result                                                                       |
| --- | ------------------------------------- | ---------------------------------------------------------------------------- |
| 1   | 1.0.0 reports 1.0.1 available         | "Version 1.0.1 is available.", with the served notes                         |
| 2   | Install completes, app relaunches     | New process, no interaction required, `passive` mode as configured           |
| 3   | Running version is 1.0.1              | Version resource reads 1.0.1; see the note below                             |
| 4   | **The database is preserved**         | Every count identical                                                        |
| 5   | Accounts present, credentials resolve | Mailbox tree, account and unread counts intact after relaunch                |
| 6   | Settings survive                      | 3 settings and the signature, unchanged                                      |

Criterion 4 in full:

    messages    1521 -> 1521      mailboxes    45 -> 45      threads  1590 -> 1590
    withBodies    48 -> 48        flagged       3 -> 3       unread    382 -> 382
    attachments    7 -> 7         accounts      1 -> 1
    newest message id 101643 -> 101643 (subject unchanged)
    settings 3 -> 3, account signature present -> present

Counts were read from the database directly rather than from the app's own UI, because the app is
the thing under test.

### The half of criterion 3 that is worth keeping

"It offered an update" is weak evidence on its own — a version comparison that always returned
true would produce it too. The useful check is the one after: the updated app, asked the same
question by the same server still offering 1.0.1, answered **"Halcyon is up to date."** The
comparison works in both directions.

### Tamper test — a modified update is refused

The signed 1.0.1 installer was copied to a `1.0.2` filename, its valid signature kept, and one
byte flipped in the middle of the file. The app offered 1.0.2, downloaded all 4,803,972 bytes of
it, and **refused to install it**. The installed version stayed 1.0.1.

This is the security argument in `src/ipc/update.rs` demonstrated rather than asserted: TLS proves
a file came from the server, the signature proves it is the file we published, and only the second
one survives a compromised release.

### What this did not test

The real GitHub endpoint. Substituting localhost exercises fetch, parse, version comparison,
download, signature verification, install and relaunch — everything except one URL string, which
is checkable by eye against a release.

**Open item:** that string is
`https://github.com/vnikie1/MailBox/releases/latest/download/latest.json`, and it must match
wherever releases are actually published. It does not affect Store builds, where the updater is
compiled out entirely.

---

## 2. Uninstall — "leaves nothing behind"

`uninstall.exe /S`, then inspected:

| Location                                 | After uninstall              |
| ---------------------------------------- | ---------------------------- |
| `%LOCALAPPDATA%\Halcyon`                 | gone                         |
| Start Menu shortcuts                     | gone                         |
| `HKCU\...\CurrentVersion\Uninstall` entry | gone                         |
| `HKCU\Software\Classes\Halcyon.eml`      | gone                         |
| `%LOCALAPPDATA%\com.uniki.halcyon`       | **kept, deliberately**       |

The app data directory surviving is intended. It holds the mail database, and an uninstaller that
silently deletes somebody's mail is a worse failure than one that leaves a folder behind. It is
not concealed, and the uninstaller does not claim to have removed it.

---

## 3. Deviation — development binaries were being shipped

**Found during this phase, on a clean install, after a full uninstall so nothing was stale.**

A release install of 1.0.0 placed these in `%LOCALAPPDATA%\Halcyon`:

    crashgate.exe     306,176
    halcyon.exe    12,123,136
    seed.exe        1,402,880
    uninstall.exe      82,108

`seed.exe` writes fabricated mail into the user's database. `crashgate.exe` crashes the app on
purpose. Every `.rs` file in `src-tauri/src/bin/` is auto-discovered by cargo as a binary of the
package, built into `target/release` by a release build, and picked up from there by the bundler.

A comment in `Cargo.toml` had asserted the opposite — that Tauri bundles only the product binary
— since Phase 3. It was wrong, and having it written down is the most likely reason nobody
checked for five phases.

**Fixed** with `autobins = false` and `required-features = ["devtools"]` on all five tools, so a
release build never produces them. `tests/bundle.rs` fails the build if either guard is removed;
both failure modes were probed.

---

## 4. Incidents

- **Two hours spent on a local HTTPS server that was never needed.** A release build refuses a
  plain-`http` updater endpoint, so a self-signed certificate for `127.0.0.1` was created,
  exported, and added to the machine's trusted roots. The real cause of the original failure was
  that the build under test predated the `dangerousInsecureTransportProtocol` line being added to
  the override config; the `--config` merge had worked the whole time. One command — searching the
  built binary for the endpoint string — would have shown this before any certificate work began.

  The certificate was removed from `Cert:\LocalMachine\Root` and `Cert:\CurrentUser\My`, and the
  `.pfx` deleted. The machine's trust store is unchanged.

- **A guess was recorded as a finding.** "The dangerous flag did not survive the `--config` merge"
  was written down as established without being checked. It was false. This is the second time in
  Phase 11 that a diagnosis was believed instead of tested — the manifest was the first, and cost
  considerably more.

- **`LNK1123: failure during conversion to COFF`** during `npm run rust:test`, after a build was
  interrupted partway through. The cause was a truncated `resource.lib` left in
  `target/debug/build/halcyon-*/out`. Deleting that one build directory fixed it. It presents as a
  linker failure and is a half-written file.

---

## 5. Import and export, driven in the running app

Built in this phase and never exercised outside the Rust integration tests. Run against a **copy**
of the real mail store: `%LOCALAPPDATA%` was redirected to a sandbox holding a copy of
`com.uniki.halcyon`, so the app opened 1,521 real messages and every change landed on the copy.
The real database was 1 account / 45 mailboxes / 1,521 messages before and after.

### Import — correct

An mbox carrying the case that separates a working reader from one that looks like it works: a
line beginning `From ` inside a message body.

| Check                                        | Result                                    |
| -------------------------------------------- | ----------------------------------------- |
| Messages imported                            | 3 of 3, all with full bodies              |
| The `From ` line did not split message 3     | 1 message from that sender, 0 fragments   |
| The quoted header survived in the body       | Present verbatim                          |
| Destination                                  | A local `local@localhost` account, mailbox named from the file |
| Existing accounts disturbed                  | None                                      |

### Export — found a data-loss bug

**"Export all mail" silently omitted the mail that had just been imported.**

Same session, same data:

- Settings opened *before* the import, then export: **45 files written, 46 mailboxes in the
  database.** No `Archive.mbox`. No error, no warning.
- Settings closed and reopened, then export: **46 files**, `Archive.mbox` present with all three
  messages and the quoted header intact.

The cause is that `startExport` iterates the `useMailboxes()` query, and an import creates a
mailbox and an account that were not there when the window opened. Nothing invalidated the
mailbox list, so the export enumerated a stale one.

This is worse than a missing refresh in a list. Somebody imports years of old mail, exports
everything as a backup, and the backup is missing exactly the mail they just imported — with a
button labelled "Export all mail" and a completion message reporting success.

**Fixed** by invalidating `keys.mailboxes` when a transfer reports finished. Verified by repeating
the whole sequence in one sitting: 47 mailboxes in the database, **47 files written**, both
imported mailboxes present and correct.

### Also fixed

"Done. 3 messages in 1 mailboxes." Importing a single mbox file is the commonest case there is,
so the one number most likely to be `1` was the one printed wrong every time. Now
"3 messages in 1 mailbox".

### Note — the file dialogs cannot be driven by messages

Five approaches failed before one worked, which is worth writing down because the next person will
try them in the same order. `SetDlgItemText` on control 1148 sets the ComboBoxEx *host* and reads
back correctly, so it looks like it worked; the dialog never sees it. `WM_COMMAND` with `IDOK`,
`BM_CLICK` on the OK button, and UIA's `ValuePattern.SetValue` all fail too, the last by timing
out and leaving the dialog wedged.

What works is genuine input: `AttachThreadInput` to take real foreground, then `SendKeys`. The
folder picker additionally needs two Enters — the first navigates into the typed folder, the
second selects it.

---

## 6. Phase 7: offline send, and killing the app mid-send

The two thirds of the Phase 7 exit gate that need no second mail account. Both run against a copy
of the mail store, sending real mail to a real address, so the evidence is what the server did
rather than what the app believes.

### A. Queued while the server was unreachable — PASS

"Offline" was simulated by pointing the account's SMTP host at `127.0.0.1:587` with nothing
listening. That is a **limitation and is recorded as one**: it produces the same classification a
dead network produces — no SMTP status, therefore `SendError::Transport`, therefore retryable —
but it does not exercise anything that depends on the OS reporting the adapter as down. The
machine's network settings, firewall and hosts file were not touched, because they are not mine to
change.

    14:03:14  send failed  id=3  connection refused (os error 10061)  state="queued"
    ~14:03:57 the SMTP host is restored
    14:04:14  outbox: sending count=1        exactly RETRY_AFTER (60s) later
    14:04:22  outbox: sent id=3

| #   | Criterion                                      | Result                                        |
| --- | ---------------------------------------------- | --------------------------------------------- |
| A1  | Never lost                                     | Row and `.eml` both survived                   |
| A2  | Never falsely reported as sent                 | State was `queued`, never `sent`               |
| A3  | The failure names its cause                    | Logged in full; the banner shows failed rows   |
| A4  | Goes out on reconnect                          | Automatically, 60s later, nothing retyped      |
| A5  | Exactly one copy delivered                     | 1 in Sent                                      |

A retryable failure returns the row to `queued`, not `failed`, so nothing is put in front of the
user for a network blip. Only after `MAX_ATTEMPTS` (5, at 60s apart) does it become a banner.

### B. Killed mid-send — PASS

The process was killed the instant the row entered `sending`, by polling the outbox rather than
sleeping a guessed number of seconds. Sleeping hits that window by luck; polling hits it every
time.

    11.4s   row 4 -> sending
            process killed
    (restart)
    14:10:25  resolving interrupted sends count=1
    14:10:29  resolved id=4 found_in_sent=false state="queued"
    14:10:38  outbox: sent id=4

| #   | Criterion                                        | Result                                     |
| --- | ------------------------------------------------ | ------------------------------------------ |
| B1  | Left genuinely in doubt, not `sent` or `failed`  | `sending`, `.eml` intact on disk           |
| B2  | Resolved by searching Sent, not by guessing      | `found_in_sent=false` from an IMAP search  |
| B3  | **Exactly one copy — never zero, never two**     | 1 in Sent                                   |
| B4  | The interruption does not spend an attempt       | `attempts=0` after recovery and resend      |

B2 is the part worth keeping. The recovery does not infer anything from local state: it asks the
server whether a message carrying that Message-ID is in Sent, and the Message-ID is the app's own
rather than the library's precisely so that question can be asked. Absent means it never left, so
requeueing cannot duplicate; present would mean it went, so marking it sent cannot lose it.

Verified across all three sends of the session: one copy each, none missing, none doubled.

### A note on Message-ID storage, which looks like a bug and is not

The outbox stores the Message-ID in header form, `<id@host>`; the `message` table stores the bare
value. A local join between the two finds nothing, which looks alarming. It is correct: the only
comparison that matters happens over IMAP, against a real `Message-ID:` header, which contains the
angle brackets. Written down because the next person to check for duplicates with a SQL join will
conclude, as this one briefly did, that no message was ever filed.

### Still not covered

That the message threads and renders correctly in Gmail, Outlook and Apple Mail — the third of the
gate that needs a person at each client.

---

## 7. Phase 7 exit gate — where it actually stands

> Exit gate: send a reply to Gmail, Outlook and Apple Mail and confirm all three thread it
> correctly and render the HTML correctly (Outlook Windows is the strictest — check it
> specifically); a send queued while offline goes out on reconnect; killing the app mid-send
> neither loses nor duplicates the message.

| Clause                                        | Status  | Evidence                                                        |
| --------------------------------------------- | ------- | --------------------------------------------------------------- |
| Sending works at all                          | PASS    | XOAUTH2; first attempt, ~7s; filed in Sent by IMAP APPEND        |
| Threads correctly in Gmail                    | PASS    | Reply and parent share one `thread_id` after the threading fix   |
| Threads correctly in Apple Mail               | PASS    | Confirmed by the account holder: iCloud shows one conversation   |
| Threads correctly in Outlook                  | **NOT TESTED** | No Outlook client available to either party              |
| Renders correctly in Outlook (the strict case) | **NOT TESTED** | As above                                                 |
| Offline send goes out on reconnect            | PASS    | §6A — retried 60s later, one copy                                |
| Mid-send kill loses nothing, duplicates nothing | PASS  | §6B — resolved against Sent, one copy, no attempt spent          |

### The two clauses that are not met, and why they are not being called met

Neither party has an Outlook client. The machine has `Microsoft.OutlookForWindows` — the *new*
Outlook, which is the web client in a window and shares outlook.com's permissive renderer. The
gate names Outlook Windows specifically because **classic** Outlook renders HTML through the Word
engine, which is where tables collapse, margins vanish and CSS silently stops applying.

Testing on new Outlook would pass easily and prove nothing about the case the clause exists for.
Recording it as untested is the honest outcome; recording it as passed because a message looked
fine in a webmail client would be worse than leaving it blank.

What can be said without a client is that the message is now well-formed in the two ways that
most often break Word-engine rendering and strict threading: it is a complete HTML document rather
than a fragment (§5), and `In-Reply-To` and `References` carry the angle brackets RFC 5322
requires. Both were wrong until this session, and both were found by looking at bytes rather than
at a rendering.

**To close these:** classic Outlook comes with a Microsoft 365 subscription. Open the message,
look at the bullets and the block quote indent, and reply to it; the reply threading either works
or it does not, and it takes about ten minutes.

---

## 8. Cold start, measured in a release build

docs/06 Phase 3 sets a budget of 800ms. What existed was 545ms **core-side, debug build, excluding
WebView paint** — the app's own log line says so in those words. The honest figure was deferred to
Phase 11 and never taken.

`tools/cold-start.ps1` measures process creation to the first moment UI Automation can find the
toolbar inside the WebView: later than first paint, earlier than fully synced, and the closest
honest proxy for "the app is up" available from outside the process.

Release build, five runs, against a copy of a real 1,521-message store:

    run 1: 1487 ms      <- first execution of a freshly built binary
    run 2:  608 ms
    run 3:  695 ms
    run 4:  627 ms
    run 5:  615 ms

    median 627 ms       budget 800 ms      WITHIN BUDGET

A sixth run with the WebView2 profile deleted — the state a new machine is in — took **659ms**, so
the 1487ms outlier is not WebView initialisation. It is a 12MB binary being paged in for the first
time, which a user pays once after installing and never again.

**The honest reading:** the budget is met at a median of 627ms. The first launch after an install
is roughly twice the budget, and no amount of averaging makes that not true; it is reported as its
own number rather than folded into a mean. This is also still not a *cold machine* — measuring
that needs a reboot between runs.

---

## 9. Settings, laid out as a form (2026-09-09)

### 9.1 Two primitives docs/02 §6 does not list

§6 specs ten components and neither a popup button nor a segmented control is among them.
Both were added: `src/ui/Select.tsx` and `src/ui/Segmented.tsx`.

The reason is not that the doc is wrong but that it was written before there was a settings
window to lay out. §6's list covers the mailbox — toolbar, sidebar row, list row, reader header,
buttons, search field, token field, attachment chip, menus, empty states — and none of those
holds a *value the user picks from a small set*. Settings is nothing but that, and the only
control the app had for it was a stack of radio buttons: six of them, twenty rows, in a window
580px tall.

Both new primitives are built on the existing token layer and add no colour of their own. The
segmented control's chosen chip is `--bg-content` over a `--fill-hover` track with a hairline,
not an accent fill, because three of them appear on the General pane at once and three accent
bars would be the loudest thing in a window whose job is to be scanned.

### 9.2 What was verified, and how

| Claim | How |
| --- | --- |
| Every control cell in a pane shares one left edge | e2e, measures `getBoundingClientRect().left` across all of them and requires one distinct value |
| Arrow keys walk the pane list, focus following | e2e, on the running page |
| Arrow keys move a segmented control | e2e, asserting `data-theme` on `<html>` after each press |
| A popup is announced once, not twice | Read back from the DOM: `select.labels` and `aria-label` agree, one each, on all four popups |
| Twelve accent swatches fit one row at the default window size | Screenshot at 780×580, both themes |
| The full gate | `npm run verify` clean; 261 unit tests, 96 e2e |

### 9.3 What is still open

- **The Composing pane holds one control.** Layout has taken it as far as it goes; the app has
  exactly one composing setting. See the note in the changelog for why `previewLines` and the
  classic layout were not moved here to fill it.
- **`assets/reference/` still has no macOS Mail *settings* capture.** The form shape here is
  from the description in docs/01 and from the platform convention both macOS and Windows
  settings follow, not from a measured reference. No claim of pixel fidelity is made.

---

## 10. The icon set (2026-09-17)

The designer's export replaced every icon the app ships. The source is `assets/brand/` (its
README says what each file is for); `npm run icon` (`tools/build-icons.ps1`) writes the rest.

### 10.1 Deviations

| Where | What the brief or docs say | What was done, and why |
| --- | --- | --- |
| docs/07 §2.4 | Generate the MSIX set from one source rather than by hand | The designer's export *is* that source: its `msix/` files already carry the `scale-`, `targetsize-` and `altform-` qualifiers, and are copied as named. The four generators that drew the Phase 0 art were removed so none can repaint it. |
| The designer's manifest snippet | Declares `uap:SplashScreen` and `Description="Mail for Windows"` | Neither used. The manifest's existing comment records why there is no splash screen (never shown for a full-trust app; declaring one failed the App Certification Kit once). The product description stays. `BackgroundColor="#EC3013"` **was** taken, as the snippet and the brief both ask. |
| Tray | — | The tray had no icon at all (see the changelog); it now uses the hand-drawn size for the display scale rather than the window icon, which is the 32px entry Tauri takes from `icon.ico`. |

### 10.2 What was verified, and how

| Claim | How |
| --- | --- |
| `icon.ico` holds the eight hand-drawn sizes, each the size its directory entry says | Parsed: 8 entries, IHDR width and height checked against every entry |
| Windows loads every size | Win32 `LoadImage` at 16, 32, 48, 64 and 256: all loaded, top-left pixel `#EC3013`. (.NET's `Icon.ToBitmap` fails on the 64 and 256 entries — and on Tauri's own generated icon from 32 up — which is a limit of .NET's reader, not of the file.) |
| The tray icons decode, at the size they are filed under | `platform::tray` unit test, through the same `Image::from_bytes` the app uses — the PNGs carry a C2PA chunk a decoder must skip |
| The right tray size for each scale | Unit test over 100–400% |
| Every image the Store manifest names exists | Listed: Square150x150, Square44x44 (scale and targetsize, plated, unplated, light-unplated), Wide310x150, Square71x71, Square310x310, StoreLogo |
| The plated assets are full-bleed red, the unplated ones transparent | Corner and top-edge pixels read from five of them |
| The installer images | Viewed: 164×314 welcome page and 150×57 header, 24-bit, the new icon on the neutral plate with the accent rule |
| The browser build's tab icon | The WebView's own target list reports `faviconUrl: http://tauri.localhost/favicon.svg` |
| The installed exe carries the new icon | `ExtractIconEx` on the installed `halcyon.exe`: 32 and 16 px, `#EC3013`. **This failed at first** — three builds linked the old `resource.lib`, because nothing told Cargo to rerun the build script for a new `icon.ico`. `build.rs` now watches it; see the changelog |

## 11. The mailbox menu (2026-09-17)

### 11.1 Deviations from docs/01 §3

docs/01 §3 lists the mailbox menu as *New Mailbox, Rename, Delete, Export Mailbox, Rebuild, Get
Account Info, Use This Mailbox As ▸*, and Favourites as *user-curated … reorderable by drag*.
The capture the menu was built against (macOS Mail on an account's inbox) shows New Mailbox, Add
to Favourites, Export Mailbox, Erase Deleted Items, Erase Junk Mail, Mark All Messages as Read,
Synchronise, Edit and Get Account Info.

| Item | Status | Why |
| --- | --- | --- |
| Everything in the capture | Built, with Rename and Delete on folders the user made | Rename and Delete are what make New Mailbox safe to offer: a menu that makes folders and cannot remove them leaves the user with every typo |
| Rebuild | Not built | Not in the capture, and in current Mail it lives in the Mailbox menu, not this one. It would drop a folder's local copy and fetch it again — the engine has that path (a `UIDVALIDITY` reset) but no command for it |
| Use This Mailbox As ▸ | Not built | Not in the capture. Reassigning roles means persisting a user override that `sync::mailboxes::persist` currently rewrites from the server on every sync; that is its own change |
| New Mailbox inside another folder | Not offered: new folders go at the top of the account | The sidebar lists an account's folders flat (`parent_id` has never been populated), so a folder made inside another would show beside it under its leaf name. Rename keeps a folder where it is, and folders made elsewhere still sync |
| Favourites reordered by drag | Not built | Favourites are appended in the order they are added (`mailbox.favourite_order`) so Ctrl+1–9 never renumber. Reordering needs a drag target the sidebar does not have |
| Removing the five default favourites | Not built | They are built rows (All Inboxes, VIPs, Flagged, All Drafts, All Sent), not stored favourites; the menu does not open on them |

### 11.2 Design decisions a reader will want the reasons for

- **Folder changes are optimistic** — standing rule 10 — and queued like moves and flags. The
  risk that comes with it is the sync: a `LIST` taken before a queued rename reaches the server
  still shows the old name. `ops::PendingTree` makes `persist` skip names with a rename or
  delete on its way, and `prune` keep names with a create or rename on its way.
- **A refusal is final.** The drain gives up on a refused mailbox change at once, puts the local
  tree back (`folders::abandon_created`, `abandon_rename`), and emits `mailbox:refused`, which
  the window shows as a toast. A refused *create* also sends mail that was moved into the
  folder back to where the server still has it.
- **Erase is `1:*`, read when the operation runs.** The store holds only the newest messages of
  any folder but the Inbox, so a UID list would leave older mail on the server. Everything
  queued before the erase runs first, so a message deleted a moment earlier is erased with the
  rest.
- **The Delete confirmation gives no count**, for the same reason: the local count can be
  smaller than what the server deletes.
- **Queued changes are pushed within about two seconds** (`SyncEngine::push_soon`). Before, the
  queue went at the start of a sync — which, with IDLE, means when the Inbox changes or at the
  five-minute safety net. docs/03 §3 describes a worker draining `pending_op`; this is it.
- **A sync no longer overwrites a flag the user changed during it** (`ops::unsent_flags`).
  This is not a change to docs/03 §3’s “on conflict, server state wins”: a flag the
  server has not been told about yet is not in conflict with the server, only newer than its
  report. When the operation is refused, it is dropped and the next sync takes the server’s
  word, as before.

### 11.3 What was verified, and how

| Layer | What | Result |
| --- | --- | --- |
| Rust unit | `sync::folders` (create, rename, delete, erase, favourites, refusals, the pending tree against `persist` and `prune`), the modified UTF-7 codec, `ops` (new operations, paths, refusal wording, sequence sets), `persist` (unsent flags), `query` (new row fields), rules and undo against a deleted folder | pass |
| Vitest | The menu's rows, order, greyed states and arguments; name checks; Favourites in the sidebar model; the browser store's folder commands | pass |
| Playwright (browser store) | `tests/e2e/mailboxMenu.spec.ts`: every row, both sheets' validation and refusals, favourites and Ctrl-numbers, erase, mark read, Settings opened on the account | pass |
| Dovecot rig | `src-tauri/tests/folders_gate.rs`, seven tests: create (plain and accented) → rename with children and mail → delete; the stale-listing race; erase of mail the store never downloaded; a refused create and a refused rename, each with the server's own words; a 20,000-UID scattered flag change; the push | 7 / 7 |
| The built app, against the rig | The release build, run with its store, logs and WebView2 profile redirected to a scratch folder, driven over WebView2's debugging port by a Playwright script; every server-side claim checked with a separate IMAP client, never through the app | 19 / 19 — see below |

The nineteen, in order: the menu on the account under All Inboxes is Mail's row for row; New
Mailbox refuses `/` as it is typed; the folder appears at once; the server has it; an accented
name reaches the server as `Re&AOc-us …` and shows as typed; Rename reaches the server; Add to
Favourites; **the favourite survives quitting and relaunching**; mail appended on the server
arrives unread; Mark All Messages as Read clears it here and on the server, and the row then
greys; Erase Deleted Items and Erase Junk Mail each empty the mailbox here and on the server; a
rename the server refuses (another client took the name) is reported in the server's words and
the folder keeps its name; Get Account Info names the account; Edit opens Settings on that
account with its name field focused; Delete removes both folders here and on the server, and the
favourite with them.

Two faults were found by this run and fixed before it passed: queued changes waited minutes for
a sync, and a flag changed during a sync was overwritten by it. Both are in the changelog.

The user's own store was not touched: its last write is the moment the installed app was closed
for the run. The window-position file the test instance wrote to was restored from a copy.

---

## 12. The rest of the mailbox menu, and the rig's certificate (2026-09-19)

### 12.1 The four things §11.1 listed as not built

| Item | Status | What it does |
| --- | --- | --- |
| Rebuild | Built | Reads the whole mailbox from the server again — every envelope and flag, removing what the server no longer has, downloading every cached body again — in a pass of its own over that mailbox, after the queue |
| Use This Mailbox As ▸ | Built | Drafts / Sent / Junk / Bin / Archive, ticked on the mailbox that has the role. The choice is kept apart from the row, so a sync cannot undo it |
| A folder inside another | Built | New Mailbox's Location lists each account and every mailbox that can hold another; the sidebar nests by path and opens and closes |
| Favourites reordered by drag | Built | The whole section, built-in rows included, by drag or with Alt+↑ / Alt+↓. A mailbox dragged in from its account becomes a favourite where it is dropped |

Two of Mail's rows in §11.1's table are still deliberately absent: the five built-in favourites
cannot be *removed* (they are rows every sidebar starts with, and the menu does not open on them),
and Rebuild is offered on the mailbox rather than in a menu bar Halcyon does not have.

### 12.2 Decisions a reader will want the reasons for

- **Rebuild reads in place; it does not drop the mailbox and fetch it again.** Mail's Rebuild
  "discards and downloads again", and dropping would be simpler — `UIDVALIDITY` recovery already
  does exactly that. But a message row here carries things no server has: a flag colour, a snooze,
  a follow-up, the junk classifier's verdict, and the row id that undo holds. Dropping them to fix
  a wrong subject would be a repair that costs more than the fault. So the envelope and the flags
  are written again over the row that is already there (`persist::Refresh::Everything`), what the
  server no longer lists is removed, and every cached body is downloaded again.
- **A rebuild is a pass over that mailbox alone** (`SyncEngine::sync_mailboxes`), not a full sync:
  on an account with a 50,000-message Inbox a full pass is minutes, and the user asked about one
  folder.
- **A rebuild waits for the queue.** A message deleted here and not yet on the server would
  otherwise be read straight back and reappear. The drain runs first, as in every pass; if
  anything queued still names the mailbox, the rebuild waits for the next one
  (`ops::names_mailbox`).
- **Use This Mailbox As is stored in `mailbox_role`, not on the mailbox row.** Every sync rewrites
  `mailbox.role` from what the server says, so a choice written there would last until the next
  one. `mailboxes::persist` reads the choice back over the server's answer, and only while the
  chosen mailbox is still listed — so a folder deleted in webmail does not leave the account with
  no Bin.
- **Gmail is not offered the choice.** Gmail decides which of its folders is which and acts on it:
  a message "deleted" into a label rather than into Gmail's Bin is not deleted at all.
- **Nesting is worked out from the paths, never stored.** `mailbox.parent_id` existed from the
  first migration and was never written; a stored parent has to be kept in step with renames on
  other devices, and the path already is the answer. `db::query::mailboxes_tree` computes it —
  **never the Inbox**, because servers that keep every folder inside it (Courier, cPanel) would
  otherwise show the whole account as the Inbox's children.
- **One table for the whole of Favourites** (migration 0014). docs/01 §3 makes the section
  reorderable, and a mailbox dragged above Flagged has to stay there; 0013's per-mailbox position
  could order the user's mailboxes among themselves and never against the rows every sidebar
  starts with, which had no row at all.
- **Where a dragged favourite will land is a line drawn over the edge of a row**, not a gap opened
  between rows: standing rule 6, the same reason a drop target here is a fill and not a border.
  The new order is shown before the core answers (standing rule 10) — a row that springs back and
  then jumps reads as a drag that failed.
- **"After" a row means "before whatever the store has next"**, which may be a row the sidebar is
  not drawing — VIPs with no VIPs. The dropped row then lands between the two rows the user saw,
  whatever sits unseen between them.

### 12.3 Three faults found while building it

- **A sync read back a message the user had removed.** A move or a delete is optimistic: the row
  goes here and the server is told at the next drain. Until then the server still lists it, and
  the next pass wrote it back as a new row — a deleted message returned, a moved one showed in
  both folders, until the change landed and a later sync tidied up. `ops::unsent_removals` is the
  removals' counterpart of `unsent_flags`, and `write_batch` skips them.
- **A mailbox emptied on another device stayed full here.** `remove_missing` will not act on an
  empty `UID SEARCH`, rightly — an empty answer is more often a fault than a fact. But `EXISTS 0`
  from the `SELECT` *is* a fact, and nothing acted on it: a Bin emptied in webmail kept every
  message here for good.
- **A test store's body cache wrote into the user's.** `fetch_body` built its cache path from
  `db::default_path`, not from the store it was given, so a rig test that downloaded a body would
  have written `bodies/1/<id>.eml` in the user's own folder, under ids that name the user's own
  messages. `Db::folder()` is the store's own directory now. Nothing had exercised it — bodies
  were fetched only by the app — but the rebuild gate does.

### 12.4 The rig's certificate, and the flags it was said to have cleared

The CA in `test/dovecot/certs.sh` lived 30 days and kept its key. That is two problems: a key on
disk that this machine would accept for **any** site, and a rig that stops working every month —
it expires on 2026-09-25. `certs.sh` now signs one server certificate, deletes the CA's key
straight after, and gives the CA critical name constraints naming only `mac-studio.local`,
`localhost`, `127.0.0.1` and `192.168.1.15`. Both last 397 days.

Checked before anything was trusted, against Windows' own chain engine with the new CA as its
only root (`hExclusiveRoot`, so no store was touched): the rig's four names validate, any other
name fails with `CERT_E_CN_NO_MATCH`, the chain expires when it should, and a certificate for a
name outside the constraints fails with `CERT_TRUST_HAS_NOT_PERMITTED_NAME_CONSTRAINT` — so the
constraints are enforced rather than decorative. The new certificate is on the rig host, staged
beside the live one; the server still serves the old certificate, because trusting a root is the
one step that should need a person. `test/dovecot/trust-ca.ps1` does that half, and refuses a CA
without name constraints.

**The flags.** A scratch probe on 2026-09-17 sent `UID STORE 2,4,…,40000 -FLAGS (\Flagged)` to the
rig's Inbox, and the changelog recorded that it may have cleared flags earlier runs had left. It
cleared nothing: every even UID up to 40,000 still carried the modification sequence it was given
when the mailbox was seeded, and a flag change that changes anything raises it. The Inbox had no
flagged message before the probe and none after.

### 12.5 What was verified, and how

| Layer | What | Result |
| --- | --- | --- |
| Rust unit | Migration 0014 against a version-13 store; `folders` (create inside another, the roles, the rebuild request, favourites and their order, a refused folder taking what was made inside it); `ops` (removals and what names a mailbox); `persist` (the refresh, removals not read back, an emptied mailbox, the rebuild's bookkeeping); `mailboxes::effective_roles`; `query` (nesting, Favourites, chosen roles) | pass |
| Vitest | The menu's new rows and the role submenu; the sidebar's tree and Favourites in the store's order; New Mailbox's locations; the browser store's nesting, roles, rebuild and favourite moves | pass |
| Playwright | `tests/e2e/mailboxMenu.spec.ts`, 41 tests: folders inside folders (made, listed, renamed, deleted, named), Use This Mailbox As (ticks, moves the role, absent on Gmail), Rebuild's two messages, and Favourites reordered by drag, by drop from an account, and by Alt+↑ / Alt+↓ — with the insertion line drawn and no row moved until the drop | pass |
| Dovecot rig | `src-tauri/tests/folders_gate.rs`, 11 tests: the seven before, plus a folder made inside another and renamed and deleted with it; a chosen Bin that outlasts a real listing and is what Erase empties; a rebuild that puts a damaged copy right and keeps what is only here; a folder emptied on the server | 11 / 11 |
| Dovecot rig, again | `dovecot_gate`, after the Inbox was re-seeded — the cold sync of fifty thousand, the killed connection, a flag changed elsewhere, the `UIDVALIDITY` reset | 5 / 5 |
| The gate | `npm run verify`: format, lint, stylelint, types, 308 unit, 154 e2e, 998 Rust | green |

| The built app, against the rig | Rebuilt, installed over the running copy, and run with its store, logs and WebView2 profile redirected to a scratch folder, driven over the debugging port; every server claim checked with a separate IMAP client | 26 / 26 |

The twenty-six, in order: the rig's 50,000 messages sync in; New Mailbox opens on the account from
a mailbox that can hold none, and inside the folder it was opened on; the folder inside appears
nested at once; the server has it as `Rig E2E …/Re&AOc-us …`; Use This Mailbox As moves the role
and the row with it; the menu stops offering Rename and Delete on it; **the choice outlasts a
Synchronise**; a flag colour is set, which no server has; Rebuild says it has started and says it
has finished; all three messages are still here; **the flag colour survived the rebuild**; the
server still has three; the folder is added to Favourites and goes to the end; the drag is
delivered with the insertion line on exactly one row; it lands above All Inboxes; Alt+↓ moves it
one place. Then, after quitting and relaunching: the favourite is where it was dragged; the chosen
role survived the restart, and another sync; the folder inside is still inside; the role is handed
back to the server's own Archive; Delete says it takes the folder inside with it, and both go,
here and on the server, and out of Favourites.

Two things about driving the app are worth keeping. **Playwright's `dragTo` never returns against
WebView2** — it asks Chromium to intercept drags (`Input.setInterceptDrags`) and this WebView2
does not answer — so the drag is dispatched as DOM events, **with a pause between them**: fired in
one task, React has not committed the state `dragstart` sets before `drop` reads it, and the
sidebar refuses a drop it does not think is happening. And the run's own helper read a folder's
name as part of its unread count, by stripping trailing digits from a row whose name ends in a
timestamp — the same trap as §11.3's, now fixed the same way, by taking the badge's own text off
the end.

The user's own store was migrated to schema 14 by the installed build on its first launch, and the
window-position file the test instance wrote to was restored from a copy.

### 12.6 Incidents

- **The new gate test emptied the rig's Inbox.** Its tidy-up step named `"INBOX"` where it meant
  the account's Bin, and a real `\Deleted` on `1:*` plus an expunge took all 50,253 seeded
  messages. The rig is disposable and `seed.sh` is deterministic, so it was re-seeded to exactly
  the corpus it started with (50,000; 45,000 read), with the index and UID list removed first so
  UIDs begin at 1 again, which the other gate's fixtures depend on. `empty_on_server` now refuses
  an Inbox outright — the refusal, not the care, is what stops it happening twice. No real account
  was involved.
- **Four doc comments lost a character.** Git Bash rewrites an argument that begins with `//` (it
  is a Windows-style switch), so block markers passed to an edit script matched one character
  late: `/// Takes back …` became `//// Takes back …` and its neighbour lost its third slash.
  Found by the compiler, fixed, and markers now travel in a file rather than in an argument.
- **The stress run's 19 failures were the machine suspending.** 820 runs of the menu spec, of
  which 801 passed; every failure landed in the two repeats around the moment the machine slept,
  the first of them `net::ERR_NETWORK_IO_SUSPENDED`, and eleven of twelve workers failed together.
  Not a race in the tests.
- **The intermittent failure of 2026-09-17 was not reproduced.** 820 parallel runs, 123 more with
  the window's source being edited underneath them (Vite reloads the page when a file changes,
  which was the likeliest cause of a single unexplained failure while that day's work was being
  written), and the gate runs since: nothing. It stays in the changelog as unexplained rather than
  as fixed.

## 13. Ready for the Store, short of the publisher's own steps (2026-10-02)

Against docs/07 §5's pre-submission checklist. What a run of this session established, and what
it did not.

### 13.1 Done and verified

| Checklist item | State | How it was checked |
| --- | --- | --- |
| Name reserved; identity copied exactly | Done (2026-08-25) | The installed package's family name is `Unikie1.HalcyonMail_anw48tyhk74bp`, character for character the one Partner Center issued |
| Version `Major.Minor.Build.0` | `1.0.0.0` | Read by `make-msix.ps1` from `tauri.conf.json`; first submission, so nothing to exceed |
| Updater disabled in the Store build | Done | `make-msix.ps1` searched the built binary for the plugin's command strings and found none |
| Store configuration warning-free | Done | `cargo clippy --no-default-features --features store --all-targets -- -D warnings`, clean after `UpdateProblem` was allowed to be unused in that build only |
| Full icon asset set | Done | Identical to `assets/brand/msix/` apart from the splash images the manifest deliberately does not use; `resources.pri` built |
| Package built | `Halcyon_1.0.0.0_x64.msix`, 9,908,354 bytes, unsigned — rebuilt 2026-10-03 00:13 with the `MinVersion` fix (§13.6); the first was 9,908,392 | `Get-AuthenticodeSignature` → `NotSigned`, which is what the Store wants |
| `MinVersion` above the Store's floor | `10.0.22000.0` | Read from `AppxManifest.xml` inside the built `.msix`; Partner Center then validated the upload: `1.0.0.0`, x64, _Windows.Desktop_ from 10.0.22000.0 |
| Installs as a real package | Done | Signed with the test certificate and installed under `WindowsApps` |
| Privacy policy live at a public URL | Live, 200, version 1.1 | <https://vnikie1.github.io/halcyon-mail/privacy.html>, regenerated from `PRIVACY.md` 1.1 on 2026-10-03 (`vnikie1/halcyon-mail` `02090a5`) |
| `runFullTrust` justification written | Done | `store/README.md` §6 |
| Screenshots — light and dark, three panes, compose, search | Done, five | 3200 × 1800, client area only; invented mail from `storedemo` |
| WACK passes with no failures | **WARNING** — 22 of 24 pass, no required failure | `wack-20261002.xml`, the same verdict as 2026-09-01: the optional *Blocked executables* (`CreateProcessW`, `ShellExecuteW`, and name matches in the binary's strings) and the *DPIAwarenessValidation* warning the tool cannot process. Run once the user approved elevation; the test package was then removed. That run was on the package before the `MinVersion` fix. **Run again on 2026-10-03 on the uploaded file itself** — a copy, test-signed and installed, so the unsigned original kept its hash (`9BA90636…`): `wack-20261003-uploaded.xml`, WARNING, the same verdict on all 24 tests. Test package removed afterwards |
| Age rating declares user-to-user communication | Done — IARC 12+, _Users Interact_ | Questionnaire 10.3 answered in Partner Center on 2026-10-03: _Communication_ app type; answers in `store/README.md` §4 |
| Personal-information access declared | Done | _Yes_, in Partner Center's Properties, with the privacy policy URL |

### 13.2 Not done, and why

- ~~**Test account credentials**~~ — entered by the user in Partner Center's _Credentials_ table
  (§13.5) the same night, since entering a password on someone's behalf is not something this work
  does. Checked after a reload: one row, named with the account's address. Its value was not read.
- **Submit for certification** — the publisher's button.
- **Clean install and uninstall on a fresh Windows 11 VM** — no VM on this machine.
- **Data paths, Credential Manager, toasts, `mailto:`, `.eml` and the startup task in a sideloaded
  install** — verified on 2026-08-31 and unchanged in kind since; not re-walked here.

### 13.3 Things the listing must not claim yet

- ~~Gmail, until the Google sign-in application is published.~~ Published the same evening,
  unverified: any Google account can sign in, through Google's warning screen, with a 100-user
  lifetime cap until the app is verified.
- **Outlook.com, until a Microsoft client is built in.** `HALCYON_MICROSOFT_CLIENT_ID` is empty, so
  the tile asks the user for a sign-in application of their own.
- **"Tested to meet accessibility guidelines"**, until Phase 10's recorded Narrator walkthrough exists.

### 13.4 Deviations

- **Screenshots carry no window frame.** docs/07 §2.7 does not ask for one, and the frame follows
  the Windows theme rather than the app's: on a machine set to dark, every light shot had a dark
  caption across its top. Client area only, for all five, so the set matches.
- **The test credentials are not in the Notes for certification.** docs/07 §2.7 says to put them
  there. Partner Center has since moved the notes to _Supplemental info → Additional Testing
  Information_, asks that the description carry no credentials, and gives them a table of their
  own on the same page. The notes point the reviewer to it.
- **No "what's new" text.** docs/07 §2.7 lists it with the listing; the form says to leave it blank
  on a product's first submission.
- **The `runFullTrust` justification is shorter than `store/README.md` first had it.** The field
  takes 500 characters; the 535-character draft was refused. The 497-character version keeps every
  fact and names Google alone, because this build has no Microsoft client.

### 13.5 Partner Center, filled in (2026-10-03)

Submission 1 of product `9ND14F638LPJ`, in the user's own Chrome, at their request. Every section
reads _Complete_; every page was saved, and the two that matter most — the listing and the notes —
were reloaded afterwards and read back field by field.

| Section | What went in |
| --- | --- |
| Pricing and availability | All worldwide and future markets; public; discoverable. Base price _USD - United States_, **0** — the form's only way to say free — for the 240-market default group. Released as soon as it passes |
| Properties | Productivity, no secondary category. Personal information: yes, with the privacy URL. Website and support email. _Record and broadcast clips_ came ticked and was unticked: games only. Accessibility left unticked |
| Age ratings | IARC questionnaire 10.3 — _Social or Communication → Communication_; blocking yes; location, purchases, reporting, moderation, friends-only no. IARC 12+, Microsoft Store 12+, PEGI _!_, ESRB _Everyone_, USK 0. The Terms of Use box ticked with the user's explicit agreement |
| Packages | The fixed `.msix`, validated; its only note is the `runFullTrust` warning |
| Store listing (en-GB) | Description (1,931 characters), 15 features, five screenshots in order with captions, box art and tile icon, 7 keywords, copyright, _Developed by_ |
| Submission options | The 497-character `runFullTrust` justification |
| Additional Testing Information | The notes for certification, without credentials; then, entered by the user, one credential row for the reviewer account |

### 13.6 Incidents

- **Every package since 2026-08-31 declared `MinVersion="1.0.0.0"`, and Partner Center was the
  first thing to notice.** `make-msix.ps1` stamped the package version with a text replacement,
  `-replace 'Version="\d+\.\d+\.\d+\.\d+"'`, which also matched the tail of
  `MinVersion="10.0.22000.0"` — from the script's first commit, `9cf4082`. Windows installed every
  such package, and in six App Certification Kit runs, 2026-08-31 to 2026-10-02, no test objected;
  the first upload was refused — _"You cannot upload msix/msixbundle/msixupload packages that targets
  Windows MinVersion <= 10.0.17134.0"_. The script now edits `<Identity Version>` through the XML
  parser, reads the staged manifest back, and stops if `MinVersion` changed or is at or below the
  Store's floor. The same read also stopped Windows PowerShell 5.1's ANSI decoding from turning
  the manifest comments' em dashes into mojibake.
- **A misdirected caption.** Typing the fourth screenshot's caption, the click meant to open its
  dialog landed after the page had scrolled, so the text was typed with nothing focused; the
  spaces scrolled the page. Every field was read back before going on: nothing had been changed.
  The remaining captions were opened by element rather than by position.
