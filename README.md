# Halcyon

A desktop email client for Windows 11, built to feel like macOS Mail.

Nothing on Windows is pleasant to use for hours a day. Mail is. Halcyon reproduces its layout,
its typography and — the part everything else rests on — its restraint: three panes, no
inspector, no ribbon, no adverts, nothing asking to be noticed.

Your mail lives on your computer. There is no Halcyon account, no server of ours between you
and your provider, and no analytics of any kind.

---

## What it does

- **Any IMAP account.** Gmail, Outlook, iCloud and Yahoo are recognised and set up for you;
  anything else takes a hostname. OAuth accounts sign in through your real browser, never
  through a window this app drew.
- **Reads offline.** Mail is downloaded and indexed locally, so search and reading work with no
  connection.
- **Search that means something.** Full text across every account, with `from:`, `subject:`,
  `has:attachment` and date ranges, over a hundred thousand messages without waiting.
- **Rules, smart mailboxes, VIPs, junk filtering** — the organising machinery Mail has, running
  locally.
- **Undo.** Archive, move, delete, flag, mark read, run rules: <kbd>Ctrl</kbd>+<kbd>Z</kbd> puts
  it back. Send has a hold you can cancel.
- **Import** from Thunderbird and mbox, **export** to mbox or a folder of `.eml` files. Your
  mail is yours and you can leave with it.
- **Keyboard throughout**, and a screen reader can drive it.

## What it deliberately does not do

- No telemetry, no analytics, no crash uploads, no "anonymous usage data".
- No account to create, no subscription, no cloud service.
- No AI features reading your mail.
- No Outlook `.pst` import yet — see [Known gaps](#known-gaps).

---

## Installing

Download the installer from [Releases](https://github.com/vnikie1/MailBox/releases), or install
from the Microsoft Store.

> **Windows may warn you the first time.** Until the certificate has been seen by enough
> machines, SmartScreen shows "Windows protected your PC" for a downloaded installer. Choose
> **More info → Run anyway**. The Store version never shows this.

Windows 11, 64-bit. The WebView2 runtime is required and is already present on Windows 11.

---

## Where your data is

| | |
|---|---|
| Mail, search index, settings | `%LOCALAPPDATA%\com.uniki.halcyon` |
| Passwords and tokens | Windows Credential Manager — never in the database, never in a file |
| Logs and crash reports | `%LOCALAPPDATA%\com.uniki.halcyon\diagnostics` |
| Window size and position | `%APPDATA%\com.uniki.halcyon` |

The database is **not encrypted**. Anything that can read your user profile can read your mail,
which is also true of every other desktop mail client. Full-disk encryption — BitLocker — is
what protects it at rest, and it is worth checking that it is on.

Uninstalling offers to remove all of it. Declining leaves the folders above untouched.

---

## Building it

```bash
npm install
npm run app:dev      # the app, with hot reload
npm run verify       # the full gate: format, lint, types, unit, e2e, rust, clippy
```

`npm run verify` must be clean before anything is committed. It will fail while `npm run
app:dev` is running, because the running app holds the binary that `cargo test` needs to
relink.

**Stack:** Tauri 2, a Rust core, React 19 and TypeScript. No CSS framework and no component
library — the interface is hand-written CSS Modules over a three-tier token layer. The UI only
ever reads and writes the local database; the Rust core owns every byte that touches a network.

`CLAUDE.md` has the working rules and the traps. `docs/` has the specifications:

| | |
|---|---|
| `docs/01` | What macOS Mail actually does, measured |
| `docs/02` | The design system — every token and component |
| `docs/03` | Architecture, the IPC contract, performance budgets |
| `docs/04` | The roadmap and its phase gates |
| `docs/05` | Risks, and what the law says about copying an interface |
| `docs/06` | The prompt library each phase was built from |
| `docs/07` | Packaging, signing and Store submission |

`CHANGELOG.md` records what was built and, more usefully, what broke and why.

---

## Known gaps

- **Outlook `.pst` import.** A `.pst` is not a mail file; it is a database of Outlook's own,
  holding messages as numbered properties with no standard message anywhere inside it. Reading
  the store is a solved problem; turning its contents back into messages is a separate piece of
  work that has not been done. Outlook can save messages as `.eml`, and an Outlook account can
  be added here over IMAP.
- **Exchange without IMAP.** Some corporate tenants disable IMAP. There is no MAPI or
  Exchange Web Services support, and there are no plans for one.
- **Pixel fidelity is unverified.** `assets/reference/` is empty, so no claim here that Halcyon
  matches Mail exactly has been checked against a screenshot.

---

## Security and privacy

- [SECURITY.md](SECURITY.md) — how to report a vulnerability.
- [PRIVACY.md](PRIVACY.md) — what leaves this machine, which is very nearly nothing.

## Licence

Copyright © 2026 Vishal Singh. All rights reserved. See [LICENSE](LICENSE).

The source is published so that anyone can check the claims above — particularly the ones about
where passwords go and what is sent over the network. That is not the same as permission to
redistribute it; see the licence for what you may do.
