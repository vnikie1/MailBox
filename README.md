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
- **Import** from Thunderbird, mbox and Outlook `.pst`; **export** to mbox or a folder of
  `.eml` files. Your mail is yours and you can leave with it.
- **Keyboard throughout**, and a screen reader can drive it.

## What it deliberately does not do

- No telemetry, no analytics, no crash uploads, no "anonymous usage data".
- No account to create, no subscription, no cloud service.
- No AI features reading your mail.

---

## Installing

**From the Microsoft Store.** That is the only way Halcyon is distributed at present, and it is
the better one: the Store signs the package, so there is no SmartScreen warning, and updates
arrive through Windows Update rather than through the app.

A downloadable installer is built and works, but is **not offered yet**. An unsigned `.exe` shows
"Windows protected your PC" on every machine that downloads it, and Smart App Control — on by
default on new Windows 11 installs — blocks it outright with no way to override. Publishing one
in that state would mean asking every user to click through a security warning, which is a habit
worth nobody's convenience. It will appear here once the code-signing certificate is in place.

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

- **Outlook `.pst` import is the least tested thing here.** It works — folders, dates, senders
  and read state all come across — but attachments are not extracted, some older messages store
  their text in a format that cannot be read, and the tests cover the file format against a real
  Outlook-produced `.pst` that contains **no mail**. The folder walk is proven against real
  Outlook output; message extraction is not. Try it on a copy before trusting it.
- **Exchange without IMAP.** Some corporate tenants disable IMAP. There is no MAPI or
  Exchange Web Services support, and there are no plans for one.
- **Pixel fidelity is unverified.** `assets/reference/` is empty, so no claim here that Halcyon
  matches Mail exactly has been checked against a screenshot.

---

## Security and privacy

- [SECURITY.md](SECURITY.md) — how to report a vulnerability.
  ([published](https://vnikie1.github.io/halcyon-mail/security.html))
- [PRIVACY.md](PRIVACY.md) — what leaves this machine, which is very nearly nothing.
  ([published](https://vnikie1.github.io/halcyon-mail/privacy.html) — this is the URL the
  Microsoft Store requires, and a dead one is an instant rejection.)

## Licence

Copyright © 2026 Vishal Singh. All rights reserved. See [LICENSE](LICENSE).

The source is published so that anyone can check the claims above — particularly the ones about
where passwords go and what is sent over the network. That is not the same as permission to
redistribute it; see the licence for what you may do.
