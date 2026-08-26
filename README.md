# macOS-Mail-quality email client for Windows 11

Planning workspace for a Windows 11 desktop email client that reproduces the look, feel and
interaction model of **macOS Mail** — because nothing on Windows is pleasant to use for hours
a day, and Mail is.

Phase 0 (the window shell) is scaffolded. See [docs/PHASE-0-VERIFICATION.md](docs/PHASE-0-VERIFICATION.md)
for what is verified, what is blocked, and why.

---

## Read in this order

| #   | File                                                             | What it is                                                                                                                                                      |
| --- | ---------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| —   | **[PROMPT.md](PROMPT.md)**                                       | **The master build prompt.** Paste into a fresh coding session to start the project. Contains the role, standing rules, definition of done, and start sequence. |
| 1   | [docs/01-macos-mail-analysis.md](docs/01-macos-mail-analysis.md) | Full teardown of macOS Mail — layout, measurements, every surface, states, motion, typography, colour, keyboard model, and what makes it feel good.             |
| 2   | [docs/02-design-system.md](docs/02-design-system.md)             | Paste-ready design tokens (colour, type, space, motion, materials) and per-component specs. The visual source of truth.                                         |
| 3   | [docs/03-architecture.md](docs/03-architecture.md)               | Stack decision, process model, SQLite schema, IPC contract, IMAP sync engine, security model, Windows integration, testing, packaging.                          |
| 4   | [docs/04-roadmap.md](docs/04-roadmap.md)                         | 12 phases with objective exit gates. ~12 weeks to v1.                                                                                                           |
| 5   | [docs/05-risks-and-legal.md](docs/05-risks-and-legal.md)         | Long-lead blockers: Gmail OAuth verification, code signing, font licensing, IMAP reality checks. **Read before Phase 4.**                                       |
| 6   | [docs/06-prompt-library.md](docs/06-prompt-library.md)           | One ready-to-paste prompt per phase, plus utility prompts for fidelity, performance and security audits.                                                        |
| 7   | [docs/07-distribution.md](docs/07-distribution.md)               | Installers, code signing, and the full Microsoft Store / MSIX submission process. **Start §2 at Phase 9.**                                                      |

---

## The short version

- **Stack:** Tauri 2 + Rust core + React 19 + TypeScript + Vite. No CSS framework, no component
  library — hand-written CSS over a strict three-tier token system.
- **Rust core** owns all protocol work: `async-imap`, `mail-parser`, `lettre`, SQLite + FTS5,
  Windows Credential Manager, OAuth 2.0 PKCE.
- **Local-first.** The UI only ever reads and writes SQLite. Every mutation is optimistic with
  a durable pending-operation queue. Deleting a message is instant.
- **Safe by default.** Bodies render in a script-free sandboxed iframe, remote content blocked,
  credentials in Credential Manager, no telemetry.
- **~12 weeks to v1.** Critical path is Phase 2 (does it look right?) and Phase 5 (does sync
  actually work?).

---

## Before you start Phase 2

Put real macOS Mail screenshots in `assets/reference/`. Without them, "pixel-perfect" is
unverifiable. Capture at minimum:

```
light-3pane.png          dark-3pane.png
light-2pane.png          dark-sidebar-collapsed.png
thread-expanded.png      compose-window.png
search-with-tokens.png   message-with-attachments.png
list-unread-and-flagged.png    contact-popover.png
```

Capture on a Retina display, note the window width in the filename, and don't scale them.

---

## Open decisions

Answer these before Phase 0 — they change the plan (see `docs/05` §9):

1. App name and icon.
2. Personal use only, or public distribution? (Changes the legal, OAuth and signing story
   substantially.)
3. Which providers must work at v1? Default assumption: Gmail + generic IMAP first, then
   Outlook and iCloud.
4. Open source or not? An OSS build cannot embed an OAuth client secret, so it must ship
   bring-your-own-credentials.
