# Halcyon — working instructions

A Windows 11 email client reproducing macOS Mail's look and interaction model.
`PROMPT.md` is the contract: role, 21 standing rules, definition of done, phase order.
Read it before doing anything substantive. The specs are `docs/01`–`docs/07`.

---

## Keep the changelog — every session, without being asked

**`CHANGELOG.md` is updated in the same session as the work, before that session ends.**
This is not optional and does not need prompting.

- Append under a `## YYYY-MM-DD — Phase N: <name>` heading. Add to today's entry if one
  exists, otherwise start one.
- Group as **Added / Changed / Removed / Fixed / Incidents / Notes**. Omit empty groups.
- Record **why**, not just what — especially for anything that reverses an earlier decision,
  deviates from a spec, or was thrown away. A reader six weeks from now needs the reasoning,
  because the code only shows the outcome.
- **Incidents are mandatory.** Anything broken, lost, reformatted by accident, or worked
  around goes in. A changelog that only records successes is not worth keeping.
- Deviations from `docs/01`–`docs/05` also go in `docs/PHASE-0-VERIFICATION.md` (or that
  phase's equivalent), which is the detailed record; `CHANGELOG.md` is the timeline.

Commit the changelog with the work it describes, not separately.

---

## Build and verify

**Smart App Control was turned off on this machine on 2026-08-25.** Builds are no longer
blocked, and `CARGO_TARGET_DIR` may be set or unset freely.

Why it had to go: cargo compiles a fresh unsigned `build-script-build.exe` whenever a new
dependency is added, or the crate name or version changes, and SAC blocked every one with
`os error 4551`. The Phase 0/1 workaround — reuse the existing `src-tauri/target` so no build
script recompiles — held only while the dependency set was frozen. It broke the moment the
crate was renamed to `halcyon`, and would have broken again at Phase 3 (rusqlite), Phase 5
(async-imap) and every release build. Disabling SAC cannot be undone without reinstalling
Windows; it was a deliberate call, not an accident.

The Phase 0 and Phase 1 guidance on target directories is now historical. See
`docs/PHASE-1-VERIFICATION.md` §6 for how those variables were untangled, and
`docs/07-distribution.md` §0 for what SAC means for end users — it is the same gate that will
block an unsigned installer on their machines.

```bash
npm run dev          # Vite only, port 1420
npm run app:dev      # Tauri + Vite
npm run verify       # the full gate: format, lint, stylelint, types, tests, rust
```

Individually: `npm run typecheck`, `npm run lint`, `npm run lint:css`, `npm run test`,
`npm run test:e2e`, `npm run rust:fmt`, `npm run rust:clippy`, `npm run rust:test`.

A change is not finished until `npm run verify` is clean. Zero lint, clippy and TypeScript
errors is a definition-of-done item, not a preference.

---

## Things that will bite you

- **Never run `prettier --write .` expecting it to skip the specs.** `.prettierignore`
  excludes `docs/`, `README.md` and `PROMPT.md` because Prettier once reflowed all of them.
- **Stylelint enforces standing rule 1.** A hex colour, `rgb()` literal, `px` length,
  duration or raw easing curve in any `*.module.css` fails the build. Add a token to
  `src/styles/tokens/` first, then use `var()`.
- **Tauri v2 needs `src-tauri/capabilities/default.json`.** Without an entry there, webview
  calls like `listen()` and `startDragging()` are denied _silently_. Review it whenever the
  IPC surface grows.
- **Windows owns the caption strip.** The window is decorated on purpose — an undecorated
  Tauri window never receives `WM_NCHITTEST` for client-area points, so Snap Layouts is
  impossible. Do not reintroduce custom caption buttons. Reasoning is in
  `src-tauri/src/platform/mod.rs`.
- **Floating layers must set their ARIA after spreading Floating UI props.** `useRole()`
  adds an `aria-labelledby` pointing at the trigger, and that outranks any `aria-label` a
  component sets before the spread. See `src/ui/Menu.tsx`.
- **`npm run verify` fails while `npm run app:dev` is running.** The running app holds
  `target\debug\halcyon.exe`, so `cargo test` cannot relink it: `failed to remove file …
  Access is denied. (os error 5)`, exit 101. It looks like a compile failure and is a file
  lock. Stop the app first.
- **`assets/reference/` is empty.** Any claim of pixel fidelity is unverifiable until real
  macOS Mail screenshots are in it. Do not assert "matches Mail" without them.

---

## Stack

Tauri 2 + Rust core + React 19 + TypeScript + Vite. Zustand, TanStack Query/Virtual,
Floating UI, Lucide, Lexical, date-fns. **No CSS framework, no component library** —
hand-written CSS Modules over the three-tier token layer. This is settled; `PROMPT.md` says
do not relitigate it.

The seam: the UI only ever reads and writes the local database over the IPC contract in
`docs/03` §4. The Rust core owns every byte that touches a network.
