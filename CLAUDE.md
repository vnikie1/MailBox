# Halyard — working instructions

A Windows 11 email client reproducing macOS Mail's look and interaction model.
`PROMPT.md` is the contract: role, 21 standing rules, definition of done, phase order.
Read it before doing anything substantive. The specs are `docs/01`–`docs/06`.

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

`CARGO_TARGET_DIR` **must point outside `C:\Users\<user>\Documents`.** Smart App Control is
enforcing on the dev machine and blocked build-script executables written under `Documents`.
See `docs/PHASE-0-VERIFICATION.md` §3.

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
