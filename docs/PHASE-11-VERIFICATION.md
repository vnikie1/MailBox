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
    newest message id 101643 -> 101643 ("Word of the day - Tact")
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
