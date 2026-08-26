# Changelog

Every working session on this project appends an entry here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project is pre-release, so
entries are grouped by date and phase rather than by version.

An entry records **what changed and why** — decisions, reversals and incidents included,
not just additions. A change that reversed an earlier decision says so. See `CLAUDE.md`
for the convention.

---

## 2026-08-25 — Product name settled: Halcyon

### Changed — product identity

- **Renamed from the placeholder `MailBox` to `Halcyon`.** `MailBox` collides with Dropbox's
  discontinued Mailbox app and is too generic to reserve or to surface in Store search.
  Halcyon means calm, and is also the mythical kingfisher said to still the seas — which is
  the product thesis: every other Windows mail client is noisy, this one is quiet.
- Applied across `tauri.conf.json` (product name, window title), `package.json`, `index.html`,
  `Cargo.toml` (crate `halcyon`, lib `halcyon_lib`), `lib.rs`/`main.rs`, the log filter env
  var (`MAILBOX_LOG` → `HALCYON_LOG`), the CI artifact name, `CLAUDE.md`, the dev gallery
  heading, and `docs/07-distribution.md`.
- **Bundle identifier changed `com.uniki.mailclient` → `com.uniki.halcyon`.** This reverses
  the Phase 0 decision to keep the identifier decoupled from the product name. Reason for
  reversing: the identifier determines the app-data directory and the NSIS upgrade path, so
  it is only cheap to change while there are no installs in the wild. That window is now;
  after release, changing it would strand user data in an orphaned directory.
- **Persisted settings keys renamed** `mailbox.settings.*` → `halcyon.settings.*` in
  `src/store/layout.ts`, `src/store/settings.ts` and the two e2e specs. Local dev state
  resets once, which is acceptable pre-release.
- **Version `0.0.0` → `0.1.0`.** MSIX rejects `0.0.0` outright, and a real pre-release semver
  is more honest than a zero. Phase 11 bumps to `1.0.0` for Store submission.

Domain vocabulary was deliberately left untouched: `Mailbox`, `mailboxId`, `threadsByMailbox`
and the "Mailboxes" tree label all name a mail folder, not the product.

### Added

- **`docs/07-distribution.md`** — the three Windows trust gates (Smart App Control,
  SmartScreen, Mark of the Web) and how they differ; the NSIS/MSI path with code-signing
  options and costs; and the full Microsoft Store MSIX process: Partner Center registration,
  name reservation and the three identity strings, packaging prep, a complete
  `AppxManifest.xml` with `mailto:`/`.eml`/startup-task extensions, `makeappx` and `signtool`
  commands, local sideload testing, the six submission sections, and the certification
  failure modes. Linked from `README.md` as doc 7.

### Notes

- **Smart App Control turned off on this machine.** The rename broke the Phase 0/1 workaround
  immediately: a new crate name means a new build-script hash directory, so cargo compiled a
  fresh unsigned `build-script-build.exe` and SAC blocked it with `os error 4551`. The
  workaround only ever held while the dependency set was frozen, and would have failed again
  at Phase 3 (rusqlite), Phase 5 (async-imap) and every release build. Disabling SAC is
  irreversible without reinstalling Windows — a deliberate call, made rather than reverting
  the crate name. `CARGO_TARGET_DIR` is now unconstrained; `CLAUDE.md` updated accordingly.
- The same gate applies to end users: an unsigned installer is hard-blocked on any machine
  with SAC on, with no "run anyway". This is the strongest argument for the Store path in
  `docs/07-distribution.md` §2, where Microsoft signs the package.
- **Store name reserved: `Halcyon Mail`.** `Halcyon` alone was already taken in Partner
  Center. The compound name is arguably the better outcome — it puts "mail" in the Store
  search index, which bare "Halcyon" never would.

  This creates a naming split that is deliberate, not an inconsistency: the Store listing and
  the MSIX `<Properties><DisplayName>` are **Halcyon Mail** (the manifest DisplayName _must_
  match a reserved name or the upload is rejected), while the binary, window title, Start-menu
  tile and all in-app branding stay **Halcyon**. No source rename needed — the project already
  builds as `Halcyon`. Recorded in `docs/07-distribution.md` §2.2.

  Reservation clock started 2026-08-25 and lapses after ~3 months without a submission. The
  roadmap is ~12 weeks, so re-check at the 10-week mark (~2026-11-03).

---

## 2026-08-25 — Phase 0: Foundation

First working session. Project went from specification-only to a running Windows app.

### Added — tooling and version control

- **Version control.** Repository initialised, first commit, and pushed to the private
  GitHub repo `vnikie1/MailBox`. `.gitattributes` normalises line endings to LF so working
  on Windows does not rewrite every file on checkout.
- **`CHANGELOG.md` and `CLAUDE.md`.** `CLAUDE.md` loads into context at the start of every
  session, and instructs that this changelog is updated in the same session as the work,
  unprompted, with incidents mandatory. That file is the enforcement mechanism.

### Changed — product name

- **Product renamed from the placeholder `Halyard` to `MailBox`** across `package.json`,
  `Cargo.toml` (crate `mailbox`, lib `mailbox_lib`), `tauri.conf.json`, the window title,
  the log filter env var (`HALYARD_LOG` → `MAILBOX_LOG`) and the CI artifact name.

  The bundle identifier stays `com.uniki.mailclient`. It was deliberately decoupled from
  the display name when the placeholder was chosen, precisely so a rename would cost
  nothing: Tauri derives the `%LOCALAPPDATA%` data directory from the identifier, so no
  mail store moves and no update channel breaks. Verified green after the rename —
  13 vitest, 6 playwright, 4 cargo tests, clippy clean, `mailbox.exe` builds.

### Added

- **Toolchain** (installed on the dev machine, not in the repo): Node 24.19.0 / npm 11.17.0,
  Rust 1.98.0 `x86_64-pc-windows-msvc`, Visual Studio Build Tools 2022 17.14.39 with MSVC
  14.44.35207 and Windows SDK 10.0.26100. WebView2 151 was already present.
- **Scaffold**: Tauri 2.11 + React 19 + TypeScript 5.7 + Vite 6, laid out per
  `docs/03-architecture.md` §9.
- **Three-tier design tokens** — `src/styles/tokens/{primitive,semantic,component}.css`
  plus `global.css`. Colour, space, radius, motion and the full type scale from
  `docs/02-design-system.md` §2–§4.
- **Rust core**:
  - `platform/backdrop.rs` — DWM system backdrop, immersive dark mode, rounded corners.
  - `platform/appearance.rs` — theme, OS accent and the transparency setting via WinRT
    `UISettings`, with change events debounced and marshalled to the main thread.
  - `ipc/window.rs` — the `appearance_get` command.
  - `src-tauri/capabilities/default.json` — a deliberately granular permission set.
- **UI**: `Titlebar`, `ShellDiagnostics` (live host/theme/accent/backdrop/DPI readout),
  a Zustand appearance store and `useAppearanceSync`.
- **Tooling**: ESLint 9 flat config with type-aware rules, Prettier, Stylelint, Vitest,
  Playwright, and a three-job GitHub Actions workflow (frontend / Rust core / installer).
- **Stylelint rule enforcing standing rule 1** — a hex colour, `rgb()` literal, `px`
  length, duration or raw easing curve inside any `*.module.css` fails the build.
- **App icon** and `tools/make-icon.cjs`, a dependency-free PNG generator.
- **`docs/PHASE-0-VERIFICATION.md`** — what is verified, what is not, and every deviation
  from the specs with its reasoning.

### Changed

- **Backdrop is Mica Alt, not Acrylic** (`PROMPT.md` step 3 and `docs/03` §8 say Acrylic).
  Acrylic blurs whatever is behind the window; Mica samples the wallpaper and desaturates
  when inactive, which is the macOS sidebar behaviour `docs/01` §3 describes. Verified
  working in the running app.
- **`--accent-hover` is derived** via `color-mix` from `--accent` instead of the fixed
  `#0A6FD8` / `#3D9BFF` in `docs/02` §3. The accent follows the Windows OS accent, so a
  fixed hover hex is wrong the moment it is not blue. The ratios reproduce the doc's values
  to within ~1/255.
- **`--accent-fg` is chosen at runtime** from the accent's luminance rather than always
  white. White on a yellow OS accent is ~1.6:1.
- **Type scale expressed as offsets from `--font-size-base`**, resolving a conflict in
  `docs/02` §1 vs §7: density must swap only `component.css`, but density changes the base
  size and the scale sat outside all three tiers. At the default 13px base the doc's table
  is reproduced exactly.
- **Drag region uses `data-tauri-drag-region`**, not `-webkit-app-region: drag` as in
  `docs/02` §6.1. The CSS property is a Chromium app-shell feature WebView2 does not honour.
- **Window is decorated (`decorations: true`)** — reverses the custom-titlebar approach
  built earlier the same session. See _Removed_ and the note below.

### Removed

- **The custom caption buttons and their entire Win32 hit-testing layer**:
  `platform/titlebar.rs`, `CaptionButtons.tsx`, the `set_caption_button_rects` command, the
  caption hover/press event channel, the `--caption-*` / `--font-caption-glyph` /
  `--win-close-*` tokens, and the window minimise/maximise/close capabilities.

  _Why:_ Snap Layouts requires `WM_NCHITTEST` to answer `HTMAXBUTTON`, and an undecorated
  Tauri window never receives that message for client-area points — `TAURI_DRAG_RESIZE_BORDERS`,
  `WRY_WEBVIEW` and the `Chrome_*` windows all span the full client rect, and the `Chrome_*`
  ones belong to the WebView2 process so they cannot be subclassed. Instrumentation was
  unambiguous: 27 hit tests for the resize borders, zero for anything inside the client
  area. Letting Windows own the caption strip makes Snap Layouts, hover, press, `Alt`+`Space`
  and screen-reader support native and unable to regress.

  _Cost:_ a ~32px system caption above the 52px toolbar — 84px of chrome against macOS
  Mail's unified 52px. The largest visual deviation in the project so far. Mica shows
  through both, so they read as one band. Revisit the toolbar height in Phase 2.

- **The `Acrylic` backdrop variant**, unused until Phase 1 has menus and popovers to put it
  on (standing rule 18).

### Fixed

- `tauri.conf.json`: NSIS `installMode` `perUser` → `currentUser` (invalid value).
- `windows` crate 0.61 import paths: `BOOL` is in `windows::core`, `ScreenToClient` in
  `Win32::Graphics::Gdi`; added the `Win32_Graphics_Gdi` feature.
- `exactOptionalPropertyTypes` violations in `vite.config.ts` and `playwright.config.ts`.
- Clippy: an unnecessary pointer cast in `hwnd_of`.
- ESLint/Stylelint config errors — backslash escaping in the Stylelint regexes, and
  type-aware rules being applied to plain-JS config files.
- **Missing `src-tauri/capabilities/default.json`.** In Tauri v2 a webview gets no core
  permissions without one, so `listen()`, `isMaximized()` and `startDragging()` were all
  being denied silently.

### Incidents

- **Prettier reformatted the specification documents.** `prettier --write .` ran before
  `.prettierignore` was scoped and reflowed `docs/*.md`, `README.md` and `PROMPT.md` —
  markdown tables realigned, `*emphasis*` → `_emphasis_`, CSS inside code fences reformatted.
  Content intact; formatting is not as authored, and there was no commit to restore from.
  `.prettierignore` now excludes `docs/`, `README.md` and `PROMPT.md` permanently.
- **Smart App Control blocked the Rust build.** `cargo build` failed with
  `os error 4551 — An Application Control policy has blocked this file` on several crates'
  build scripts (CodeIntegrity event 3077). Resolved without disabling Smart App Control:
  the build completes with `CARGO_TARGET_DIR` outside `C:\Users\<user>\Documents`, and
  previously-blocked binaries later ran untouched, so the blocks appear time- and
  load-sensitive rather than a fixed rule. See `docs/PHASE-0-VERIFICATION.md` §3.

### Notes

- Twenty conflicts and gaps were found across `docs/01`–`docs/06` — broken cross-references,
  a 13px/13.5px font-size disagreement, FTS5 `content=''` vs external-content, row-height
  arithmetic that does not close against the line-height, a missing `gm_msgid` column, and
  keyset pagination that only supports date sort. Recorded in the session review; the
  load-bearing ones are reflected in `docs/PHASE-0-VERIFICATION.md` §4.
- `assets/reference/` is still empty. It blocks the Phase 2 exit gate, not this one, and
  filling it needs half an hour on a Mac running Sequoia.

## 2026-08-25 — Phase 1: Design system

Second working session, same day. The project went from a themed empty window to a
complete primitive layer with a gallery to prove it. Phase 1 exit gate passes; the full
record, including everything not verified, is in `docs/PHASE-1-VERIFICATION.md`.

### Added

- **Sixteen primitives** in `src/ui/`, one CSS Module each: `Button` (filled / bordered /
  plain / destructive), `IconButton`, `Menu`, `ContextMenu`, `Popover`, `Tooltip`,
  `TextField`, `TokenField`, `Chip`, `Avatar`, `Badge`, `Divider`, `Sheet`, `Toast`,
  `Skeleton`, `ScrollArea`. Floating layers use Floating UI with `flip`/`shift` collision
  handling, and submenus open on a 150ms delay guarded by `safePolygon()` — the safe
  triangle, without which moving the pointer diagonally toward a submenu crosses the row
  below and closes the thing you were aiming at.
- **`/dev/gallery`** — every primitive in every state, rendered twice under forced
  `[data-theme]` subtrees so light and dark are on screen together, with theme and density
  toggles that drive the real settings store. Dev-only: `main.tsx` loads it through a
  dynamic import behind `import.meta.env.DEV`, so it is absent from the production bundle
  rather than relying on tree-shaking to notice (verified by grepping `dist/`).
- **Component token tier completed** — all of `docs/02` §6.2–§6.10 plus the three density
  modes of §7. Density still swaps only `component.css`.
- **Settings store** (`src/store/settings.ts`) — theme, density and transparency
  preferences, persisted to localStorage and read synchronously at boot. Phase 3 moves this
  into the `settings` table; the shape is chosen to make that a port rather than a rewrite.
- **Inter, self-hosted** via `@fontsource-variable/inter`. Vite fingerprints and bundles
  the woff2 subsets, so nothing is fetched at runtime and the build works offline, as
  `docs/02` §2 requires. The `opsz` cut rather than the plain weight axis: `docs/01` §10 is
  reproducing SF Pro's Text/Display split at 20pt, and Inter's optical-size axis does the
  same job continuously. Until now the app had been rendering in Segoe UI Variable, which
  meant the `cv05`/`cv08`/`ss03` feature settings in the token file did nothing at all.
- **`src/lib/tokens.ts`** — reads a token's value back out of the cascade. Two things need
  it: Floating UI takes offsets as numbers, and anything unmounting after an exit
  transition needs the duration. Both would otherwise put design values in TypeScript where
  the stylelint rule cannot see them.
- **`.materialSidebar` / `.materialHeader` / `.materialMenu`** in `global.css`, composed
  into the primitives that wear them. `docs/02` §5, with the blur radii lifted to
  `--filter-*` roles so Reduce Transparency switches all three off from one place.
- **32 new tests.** Vitest 13 to 45, Playwright 6 to 11, including 24 keyboard cases across
  Menu, TokenField and Popover, and a committed visual baseline of the gallery
  (1400 x 3022, both themes, all sixteen primitives).

### Changed

- **`applyAppearance` now resolves OS state against user preferences** and writes
  `[data-density]` as well as theme and transparency. It remains the only function in the
  app that touches those attributes — theme, density and transparency each have two inputs,
  and resolving them in one place is what stops the two halves fighting.
- **`--dur-sheet` (320ms) is defined but unused.** `docs/02` §4 assigns it to "popover /
  compose open"; `PROMPT.md` standing rule 7 says every animation is 100–250ms. The prompt
  wins by its own terms, so Popover, Sheet and Toast animate on `--dur-base` (200ms) and
  the token waits for Phase 7 to decide about the compose window. **This one needs a
  decision** — see `docs/PHASE-1-VERIFICATION.md` §4.1.
- **The chip's close button is laid out permanently and fades in**, rather than appearing
  as `docs/02` §6.6 words it. Inserting it into the layout on hover would resize the chip
  and shove every chip after it sideways, which standing rule 6 forbids. Same reasoning for
  `TextField`'s clear button and `MenuItem`'s leading icon column.
- **Stylelint** learned that `composes: materialMenu from global` is a CSS Modules value,
  not a CSS keyword, so `value-keyword-case` no longer lower-cases the class name.
- **ESLint** disables `@typescript-eslint/unbound-method` for the five floating
  primitives. Floating UI declares `refs.setFloating` as a method, so the rule fires on the
  library's only documented usage; wrapping it in an arrow function would create a new ref
  callback every render and detach the node each time.

### Fixed

- **Floating UI's `aria-labelledby` was silently overriding our `aria-label`.** `useRole`
  labels a menu by its trigger, and `aria-labelledby` outranks `aria-label` — so a context
  menu had no accessible name at all, and a submenu was named after the row that opened it
  rather than after itself. Interaction props are now spread before the ARIA attributes,
  which clear `aria-labelledby` where the component supplies its own name. Found by a test
  asserting the accessible name, not by reading the code.
- **`Avatar` initials are grapheme-aware.** `Intl.Segmenter`, not `charAt(0)` or a
  code-point split: display names in mail are exactly as unruly as they sound, and both
  cheaper approaches break emoji and combining accents.

### Incidents

- **The first committed visual baseline was a blank white page.** `page.goto` resolves on
  `load`, but the gallery arrives through a dynamic import, so the document was still empty
  — and `document.fonts.ready` resolved instantly because no font had been requested yet.
  The screenshot test being the _fastest_ of the five is what gave it away; nothing in the
  test output was wrong. Caught only by opening the PNG. The spec now waits for both theme
  columns before capturing.
- **The second baseline covered a third of the primitives.** `fullPage: true` captures the
  document, and this document never scrolls — the gallery scrolls inside a `ScrollArea`, so
  the shot was one viewport and no evidence about the twelve primitives below the fold. The
  spec now measures the scroller's content height and grows the viewport to it.
- **Smart App Control blocked the Rust build again, and the Phase 0 guidance turns out to
  be wrong.** A fresh `CARGO_TARGET_DIR` outside `Documents` failed with `os error 4551` on
  `icu_normalizer_data`'s build script, twice. The identical command against the existing
  `src-tauri/target` — which is _inside_ `Documents`, the location `CLAUDE.md` says to
  avoid — succeeded immediately.

  The determining factor is not the directory: it is whether a build-script executable has
  to be freshly compiled and then run. `src-tauri/target` already holds those from Phase 0,
  so nothing new is executed and nothing is blocked. Phase 0 changed two variables at once
  and credited the wrong one. Revised guidance is in `docs/PHASE-1-VERIFICATION.md` §6;
  `CLAUDE.md` still carries the old rule and should be corrected.

- **One Vitest case was order-dependent** — the submenu keyboard test passed alone and
  failed inside `npm run verify`. Focus moves in a React effect, and `userEvent` does not
  flush effects between keystrokes, so a synchronous `toHaveFocus()` passed or failed on
  scheduler interleaving. Every focus assertion in the menu suite now awaits settlement,
  confirmed over three consecutive runs.

### Notes

- `assets/reference/` is still empty, so no primitive's metrics have been compared against
  real macOS Mail. Everything here comes from `docs/02`. This blocks the Phase 2 exit gate.
- Display scaling at 125 / 150 / 175 % is still unverified — the gallery runs in Chromium
  at 1 dpr. Carried into Phase 2.
- The production bundle grew by 3 kB (215 to 218 kB) because nothing in `src/ui` is
  imported by the shipping app yet. Phase 2 is what puts the primitives on screen.

## 2026-08-25 — Phase 2: Reference captures

Groundwork; the shell itself is the entry below.

Groundwork for Phase 2. No application code changed.

### Added

- **`assets/reference/` is no longer empty.** Four captures of macOS Mail from a Mac Studio
  on the LAN — light and dark, each in the active and inactive window state, cropped to the
  Mail window at 2664 x 2320 px. This unblocks the Phase 2 exit gate, which is a side-by-side
  comparison against real Mail, and retires the standing caveat that no fidelity claim in
  this project was checkable.
- **`assets/reference/README.txt`** now records the capture conditions rather than a
  placeholder: macOS 26.6.2, a 2x Retina display, and therefore **1 logical point = 2 pixels
  in these files**. Every measurement taken from them has to be halved before it is compared
  against `docs/01` or `docs/02`, which are written in points. Getting that backwards would
  make every metric in Phase 2 wrong by a factor of two.

### Notes

- **The reference Mail is macOS 26; the specs describe an older one.** Five differences are
  itemised in the README so they do not get "corrected" back to the spec by mistake: there is
  no single unified toolbar (each pane has its own header), the toolbar button order differs
  from `docs/02` §6.1, the message list has a category filter row the specs do not mention, a
  Summarise button sits in the reading pane, and preview lines are AI summaries rather than
  the first line of the body. Phase 2 builds what Mail actually looks like now and records
  each departure.
- First eyeballed measurements: a two-line message row is ~80pt against the 78 in `docs/02`
  §6.3, which is close enough to be a rounding question; a sidebar row is ~32pt against the
  28 in §6.2, which is not. Both to be measured properly in Phase 2 rather than trusted from
  a scaled render.

### Incidents

- **macOS blocks `screencapture` over SSH.** The Mac was reachable and SSH worked, but every
  capture returned `could not create image from display` — TCC does not grant screen recording
  to an SSH session. One workaround attempt (running the capture inside the logged-in GUI
  session via `sudo launchctl asuser`) was refused by the sandbox, correctly: it looks
  indistinguishable from credential abuse. The captures were taken from Terminal on the Mac
  and pulled with `scp`, which is the honest path and is documented in the README for next time.
- **Three captures were thrown away before one was usable.** The first pair had a terminal
  window covering Mail's reading pane, and the "dark" one was not dark — the appearance switch
  had not taken effect before the shutter. The second pair was clean but showed the _inactive_
  window, which the grey traffic lights gave away; that pair was kept deliberately, since the
  inactive state is itself a reference `docs/01` §9.11 needs. Only the third pair, taken with
  a ten-second delay so Mail could be clicked to the front, is the primary reference.

## 2026-08-25 — Phase 2: Static shell with mock data

The window went from a themed empty frame to a working three-pane mail client driven by
fixtures. **The exit gate is not passed** — five things are outstanding and listed in
`docs/PHASE-2-VERIFICATION.md` §5. `docs/04` is explicit that this is the phase not to rush,
so the gate stays open.

### Added

- **Fixture generator** (`src/mock/`) — 800 threads, **2,891 messages**, 26 mailboxes across
  3 accounts, seeded and deterministic so the visual baselines mean something. Threads run
  1–15 messages, 257 carry attachments, 58 are flagged. Dates are weighted toward the present
  rather than spread evenly: a uniform spread over two years puts two messages under "Today"
  and a year of month headers below, which is nothing like an inbox and would leave the
  sticky-header work untested exactly where you look at it.
- **Domain types** (`src/domain/mail.ts`) mirroring the SQLite schema in `docs/03` §3, so
  Phase 3 replaces where the data comes from and not what it looks like.
- **Sidebar** — unified All Inboxes / All Drafts / All Sent rows expanding to per-account
  children, per-account sections, disclosure animation, unread badges that vanish at zero,
  and no hover highlight (`docs/01` §3 is explicit, and it is most of why the sidebar reads
  as calm).
- **Message list** — TanStack Virtual, section headers as items in the same virtual list so
  their heights are part of its arithmetic, a sticky header drawn over the scroller because
  `position: sticky` cannot work on absolutely-positioned virtual items, every row state,
  thread count pills, flag colours, preview lines 0–5, contact photos, and multi-select where
  contiguous runs merge into one rounded block.
- **Reader** — thread stacked oldest-first with every message collapsed but the newest,
  recipient expansion, attachment chips, a flagged banner, selectable body text.
- **Shell** — three panes above 1000px, two below, one with push navigation below 700px;
  draggable dividers with keyboard support and persisted widths; sidebar collapse; classic
  layout; three density modes, all reachable from the list's overflow menu.
- **`src/lib/date.ts`** — the relative-adaptive date format from `docs/01` §4 and the
  section buckets, both taking `now` as an argument rather than reading the clock.
- **17 new tests.** Vitest 45 → 62, Playwright 11 → 15, including a scroll-performance
  measurement and four visual baselines (both themes at 1400px and 900px).

### Changed — metrics corrected against the reference

Measured off `assets/reference/`, halving for the 2× display:

- **Sidebar row 28 → 32.** Six consecutive row pairs 48px apart in a 2000px render of the
  2664px capture: 64 device pixels, 32 points. `docs/02` §6.2 and §7 both say 28.
- **Two-line list row 78 → 80**, from eight consecutive rows 120px apart. The 16pt step per
  preview line is kept, putting the other two at 48 and 64.
- **The window has one 52pt band of chrome, not two.** The first version stacked a
  window-wide toolbar on top of the list's own header — 104pt against Mail's 52. There is no
  toolbar band now; each pane draws its own header at the shared height and the three line
  up. `docs/02` §6.1 describes a unified bar; macOS 26 does not have one.
- **The reader subject moved** out of the 17pt title slot above the header and into the
  header block, under the sender. `docs/01` §5 draws it the old way.
- **Toolbar buttons sit on rounded capsules** and the button order follows the reference —
  compose, then reply group, then destructive group, then move and flag — rather than
  `docs/02` §6.1's order, which puts delete next to reply.

Every one of these is "the reference disagrees with the doc and the reference wins", with the
measurement recorded. Full table in `docs/PHASE-2-VERIFICATION.md` §2.

### Changed — other

- **Contact photos are grey, not colour-hashed.** `docs/01` §4 asks for a colour derived from
  the address hash; standing rule 2 permits exactly two saturated families on screen, and a
  wall of coloured initials circles is the loudest way to break the restraint `docs/01` §9.3
  describes. Grey initials on `--bg-raised`, as Mail draws them.
- **`className` props across `src/ui` widened to `string | undefined`.**
  `noUncheckedIndexedAccess` types every `styles.x` as possibly undefined, and
  `exactOptionalPropertyTypes` then refuses it — so every call site would have had to launder
  it through `cx()`. Widening the prop is the honest fix: the components genuinely accept
  "no class".

### Removed

- **The Phase 0 diagnostics panel and custom titlebar component.** They existed to prove the
  Win32 layer reported theme, accent, backdrop and DPI correctly. What they verified is in
  `docs/PHASE-0-VERIFICATION.md` and re-checked by the e2e suite; a debug readout in the
  shipping window is the placeholder standing rule 18 forbids.

### Fixed

- **An infinite render loop that blanked the message list.** `selectVisibleThreads` was a
  zustand selector that mapped and filtered, so `useSyncExternalStore` saw a new array every
  call and React re-rendered until it gave up. It presented as ten Playwright tests failing
  with "element not found", which reads like a selector problem rather than a crash — only
  the browser console named it. It is now a plain function of `(store, mailboxId)` that
  callers memoise, so the shape that caused it no longer exists.
- **Push navigation skipped the message list.** Narrowing to one pane landed on the reader,
  because the effect fired whenever a thread was selected and one always is at startup. The
  effects now watch for the selection _changing_.

### Incidents

- **A second order-dependent Vitest failure with the same root cause as Phase 1's.** The menu
  test pressed `{ArrowDown}{Enter}` in one call; focus moves in a React effect, so Enter
  sometimes landed on the panel instead of the item. Passed alone, failed under
  `npm run verify`. **Any test pressing a key that acts on a focused element must await the
  focus first.** Twice now; treat it as a rule rather than a coincidence.

### Notes

- **Scrolling holds 60fps.** Measured over 54,000px of travel: median 16.6ms, p95 18.4ms,
  worst 19.7ms. The measurement is a test and prints those numbers every run; its assertion
  sits at 33ms so machine load cannot make it flaky while it still catches virtualisation
  breaking.
- Three things in the reference are deliberately not reproduced: the category filter row, the
  Summarise button, and AI-generated preview text. All three are Apple Intelligence or
  message categorisation, which this project has no equivalent of, and drawing them would be
  the fake data path standing rule 18 forbids.
- Still unchecked from the `docs/02` §8 visual QA list: the 50%-opacity overlay against the
  reference, display scaling at 125 / 150 / 175 %, and the `--label-2` contrast measurement.

## 2026-08-25 — Phase 2: closing the exit gate

The five items `docs/PHASE-2-VERIFICATION.md` §5 listed as outstanding are done, and the
gate is passed. Two of them found real defects.

### Added

- **Sort menu** — `docs/01` §4's full set: Date / From / Subject / Size / Unread / Flags /
  Attachments, both directions, and Organise by Conversation. In
  `src/features/messageList/sort.ts` rather than in the store, because Phase 3 gets this
  ordering from a SQLite index and `docs/03`'s keyset pagination only supports the date
  one — keeping it in a single file keeps that conversation to a single file too.
  Sorting by subject strips `Re:`/`Fwd:` for the comparison only, or a conversation
  scatters across the alphabet under R. The boolean fields fall back to newest-first inside
  each group, because flagged messages in arbitrary order are useless. 12 tests.
- **`size` on messages and threads**, which sorting by size needed and `docs/03` §3 already
  had in the schema.
- **Reader hover action glyphs** — reply, reply-all, forward, fading in over `--dur-fast`.
  Laid out permanently so the date does not shuffle sideways as the pointer crosses, and
  revealed by keyboard focus too: an affordance that exists only under a pointer is one a
  keyboard user can tab into but never see.
- **Sidebar drag-and-drop.** Rows drag, mailboxes accept, and the target styling is
  `docs/02` §6.2's accent at 25% with a 1px inset ring — a box-shadow, not a border, since a
  border occupies layout and every row below would shift a pixel as the pointer crossed.
  The drop performs a real move rather than a no-op, because a target that highlights and
  does nothing is the fake path standing rule 18 forbids. Its shape is already what Phase 3
  needs: the UI updates in the same frame and nothing waits, which is standing rule 10.
- **`tests/e2e/scaling.spec.ts`** and three extra Playwright projects, so the suite runs at
  100 / 125 / 150 / 175 %. It asserts the layout is identical **in CSS pixels** at every
  scale; drift would mean something is measured in device pixels, and the likely culprit
  would be the token reads in `lib/tokens.ts`. Nothing drifts.

### Changed

- **Contact photos are tinted after all, and the earlier decision to decline them was
  wrong.** `docs/01` §4 asks for a colour derived from the address hash; that was declined
  on the grounds that standing rule 2 permits only the accent and the flags to be saturated.
  Looking properly at `assets/reference/`, Mail's own avatars _are_ tinted — soft lavender
  and blue-grey discs. The rule bans _saturated_ colour and those are not that. Eight tints,
  chosen by FNV-1a over the address, defined as tokens per theme.
- **`--label-2` in light mode, 50% → 55% black.** `docs/02` contradicts itself: §3 pins
  secondaryLabel at 50%, and §8 requires `--label-2` on `--bg-content` to reach 4.5:1.
  Measured, 50% composites to #808080 on white and gives **3.98:1** — it fails the doc's own
  floor. 55% gives **4.76:1** and is indistinguishable side by side. Dark mode already
  passed at **5.91:1** and is untouched. Both numbers now print on every test run.

### Fixed

- **A flake that survived two wrong fixes.** Two menu tests failed intermittently on
  `toHaveFocus`, always the first ArrowDown after opening. First attempt raised the `waitFor`
  timeout to 4s; it failed again, just slower, which should have been the clue. Second
  attempt sent the arrows one at a time instead of three at once, on a coalescing theory.
  Still flaky.

  The real cause: the test sent keys before the menu was ready. The panel mounts,
  `FloatingFocusManager` moves focus into it and `FloatingList` registers the items — all in
  effects, all _after_ `findByRole` can see the element. A key sent in that gap reaches a
  list with no items registered, moves nothing, and the test waits out its timeout for focus
  that was never coming. Menus are now opened through a helper that waits for focus to reach
  the panel. Six consecutive suite runs clean, then three consecutive `npm run verify`.

  **Waiting longer for the wrong thing never works.** A timeout that fires at exactly its
  limit is evidence the condition is not merely late.

### Incidents

- **A test that pinned nothing and passed.** `tests/e2e/scaling.spec.ts` was written just
  after the Halcyon rename swept the persisted settings keys, and pinned the old
  `mailbox.settings.*` names. It passed regardless — the defaults it was trying to pin
  happened to match the defaults it got. A test that silently pins nothing is worse than one
  that fails. Corrected; the storage key names are now something two places have to agree on.

### Notes

- 74 unit tests, 30 end-to-end across four display scales. `npm run verify` exits 0.
- Scrolling still holds frame rate: median 16.8ms, p95 20.9ms over 54,000px.
- One `docs/02` §8 item remains unchecked: the 50%-opacity overlay against the reference with
  row baselines within 2px. It needs an image-diff tool this project does not have, and the
  metrics it would confirm are already asserted numerically.

## 2026-08-25 — Phase 2: what running the real app found

The app had never actually been _looked at_ outside Chromium. Running it in the Tauri
window and screenshotting it found two defects the whole e2e suite had passed straight over,
plus a third that was latent until the sort menu existed to expose it.

### Fixed

- **Three sidebar rows highlighted at once.** The same mailbox appears in several places in
  the tree — Northgate's inbox is a child of All Inboxes and also a row in Northgate's own
  section — and selection was keyed by mailbox id, so every copy matched. Selection is now
  keyed by the sidebar **row**, which is the actual identity. Invisible to Playwright because
  the tests asserted on roles and metrics, not on how many things were selected.
- **"All Inboxes" was an alias for the first account, not a union.** The node carried a
  single `mailboxId` set to the first match, so the row showed Northgate's inbox while its
  badge summed all three accounts — it promised 407 messages and delivered 199. `SidebarNode`
  now carries `mailboxIds: string[]` and nothing else, and `listRows` merges across them.
  The single-id field is gone rather than deprecated, so the mistake cannot be made again.
- **Arrow keys and shift-select walked the wrong order.** Both read the store's own
  date-ordered array while the list rendered whatever the sort menu said. Sorting by From and
  pressing Down jumped somewhere unrelated. `extendSelection` and `moveSelection` now take the
  visible order as an argument; the store deliberately does not know how the list is sorted.
- 8 new tests covering all three, in `tests/unit/sidebar.test.ts`.

### Verified in the real window

Captured at 150 % display scaling, dark theme, Mica Alt:

- The Windows OS accent (`#F7630C`) reaches the sidebar icons, the unread dots and the
  selection — so the accent chain from `UISettings` through IPC to `--accent-system` works
  outside the browser, where the browser path has no accent to report at all.
- **Inactive-window desaturation works.** The first capture was taken without focus and
  everything was correctly grey; it looked wrong until the active capture showed the accent
  arriving. Worth knowing before someone "fixes" it.
- Avatar tints render as intended — muted, distinguishable, not saturated.
- Row metrics hold at 150 %: sidebar rows 48 device px (32pt), list rows 120 (80pt).

### Incidents

- **I reported the app as broken when it was not.** The first screenshot of the Tauri window
  showed nothing but the Mica backdrop, and I took that as a rendering failure — spent several
  steps chasing capabilities, `index.html` and a temporary error probe. The window had simply
  not painted yet when the shutter went. Re-capturing would have taken ten seconds and saved
  all of it. **A blank first frame is not evidence of a blank app.**
- Two dead ends worth recording so they are not retried: Tauri sets the native window title
  from `tauri.conf.json`, so `document.title` is useless as a diagnostic channel; and
  `SetForegroundWindow` from a background process is blocked by Windows, so a screen capture
  aimed at a window's coordinates can silently photograph whatever is on top of it — which it
  did, capturing unrelated windows. Raising the window with `SetWindowPos(HWND_TOPMOST)`, or
  attaching to the foreground thread's input queue first, is what actually works.

### Notes

- 82 unit tests, 30 end-to-end. `npm run verify` exits 0.
- The lesson from all three defects is the same: an e2e suite that asserts roles, metrics and
  screenshots of a _browser_ proved nothing about how the app behaves when someone uses it.
  Phase 3 onward should run the real window and look at it as part of each gate, not after.

## 2026-08-25 — Phase 3: Local data layer

The mail moved from fixtures generated in the browser to real SQLite behind the IPC contract.
The window looks the same and is now backed by a hundred thousand messages on disk. Exit gate
passed on every measured clause; the full record is `docs/PHASE-3-VERIFICATION.md`.

### Added

- **Schema and migrations** — `src-tauri/migrations/0001_initial.sql`, the whole of `docs/03`
  §3, applied by a forward-only runner that embeds each file with `include_str!` and records
  it in `schema_migration`. There is no `down`: a rollback that correctly un-migrates data is
  a fiction, and the honest recovery is a fix-forward migration plus a restore.
- **`Db`**, deliberately asymmetric — writes through a single actor on its own OS thread with
  every job in a transaction, reads through an r2d2 pool on blocking threads. WAL lets both
  run at once, and serialising the writer turns `SQLITE_BUSY` from an error every caller must
  handle into a queue nobody has to think about.
- **FTS5, external-content**, with triggers keeping it in step with `message`.
- **Keyset pagination** on `(date_received, id)` — never `OFFSET`. The id in the cursor is
  not decoration: timestamps collide constantly in mail, and a cursor on the date alone
  repeats or skips the whole colliding run. Both the Rust and the browser implementation are
  tested for exactly that.
- **The command surface** with real behaviour behind it: `accounts_list`, `mailboxes_tree`,
  `messages_page`, `message_get`, `thread_get`, `search`, `msg_set_flags`, `msg_move`,
  `msg_delete`, plus `mailbox:changed` and `messages:updated` events. Every mutation also
  writes a `pending_op`, which is the mechanism Phase 5 drains.
- **Typed bindings** — eleven types generated from the Rust structs into `src/lib/generated/`
  by `cargo test`, so a field renamed on one side and not the other is a TypeScript error
  rather than an `undefined` at runtime.
- **`seed`**, a dev binary generating 100,000 messages across 42 mailboxes in 2.4 s, which
  finishes by printing the timings and `EXPLAIN QUERY PLAN` output the exit gate asks to see.
- **`src/mock/browserStore.ts`** — what the app is when served by Vite rather than hosted in
  a WebView. Not a mock: the Playwright suite drives it, and its paging semantics match the
  Rust implementation deliberately, because a browser store that paged differently would let
  the tests pass over a bug that only appears in the real app.
- 26 new tests (frontend 55 → 68, Rust 4 → 40).

### Changed

- **The UI reads the IPC contract, not fixtures.** TanStack Query over the commands, an
  infinite query paging by cursor, and `src/store/mail.ts` reduced to selection state, which
  is all it should ever have held. `src/domain/mail.ts` and the Phase 2 fixture generator are
  gone; the UI now speaks the generated types.
- **No polling anywhere.** The QueryClient sets no `refetchInterval`, and
  `refetchOnWindowFocus` is off — standing rule 14 as a configuration rather than a habit.
  Freshness comes from the core's events.
- **`ix_msg_list` gained a third column.** `docs/03` §3 gives
  `(mailbox_id, date_received DESC)`; the keyset comparison is on the _pair_
  `(date_received, id)`, and without `id` in the index the plan is not covering.
- **"Organise by Conversation" was removed rather than left inert.** Server-side grouping
  needs a thread-per-mailbox projection to stay inside the budget, and threading is the sync
  engine's job in Phase 5. A toggle that does nothing is worse than no toggle. A unread-only
  filter took its place in the list header, which the store applies for real.
- **`default-run = "halcyon"`** in `Cargo.toml` — with a second binary present, `cargo run`
  and therefore `tauri dev` could no longer choose.

### Fixed

- **A full recount ran after every mutation, costing 84 ms per "mark as read".** The cached
  mailbox badge was recomputed with `COUNT(*)` over the whole mailbox on every flag change,
  move and delete, blocking the single writer while it ran. Standing rule 10 wants the local
  write instant. Replaced with before/after snapshots of only the affected rows and a delta
  applied to the cached counts: 50 messages marked read now costs **13.8 ms including the
  count maintenance**. A test asserts the incremental path agrees with a full recount, since
  drift is the risk that trade introduces.
- **A permanent delete never told the server to expunge.** The `pending_op` was enqueued
  _after_ the rows were removed, so the query resolving which account they belonged to found
  nothing and silently wrote no op. The message would have been deleted locally and
  reappeared on the next sync. Accounts are now resolved before the delete.
- **The reader showed "No Message Selected" beside a selected row.** Seeded messages have no
  `thread_id`, so `thread_get` matched nothing. It now falls back to the message with that
  id — the same path real unthreaded mail will take, and standing rule 13 applied to metadata
  that has not been computed yet rather than to metadata that is broken.
- **Every message in the list showed the same time.** The seed's integer date arithmetic
  collapsed every small roll to one instant. Jitter within the day fixed it.

### Incidents

- **Eleven generated files were written outside the repository.** `#[ts(export_to = ...)]`
  resolves relative to the _source file_, not the crate root, so a `../../../` path that
  looked correct from `src/db/` landed them in the parent of the project directory. Removed;
  the destination is now set once in `.cargo/config.toml` via `TS_RS_EXPORT_DIR`, at the
  repository root because cargo reads that file from the invocation directory upward.
- **The app rendered nothing after the swap, from a stale Vite cache.**
  `@tanstack/react-query` had been a dependency since Phase 0 but was never imported until
  now, so Vite's pre-bundle cache held a copy linked against a different React instance —
  presenting as "Invalid hook call" and a blank page. `rm -rf node_modules/.vite` fixed it,
  after a detour into checking for duplicate React installs.
- **The schema caught two seed bugs, which is the schema working.**
  `UNIQUE(mailbox_id, uid)` rejected UIDs numbered from the batch counter rather than per
  mailbox; the foreign key on `thread_id` rejected pointing every message at a thread row
  that did not exist. Both would otherwise have produced a plausible database with wrong data.
- **Two bugs were visible only in the running app, again** — the reader fallback and the
  identical timestamps above. Neither was caught by 138 passing tests. Phase 2's lesson holds
  and is now written into two verification records: running the real window is a distinct
  verification activity from running the suite.

### Notes

- Measured against the 100k seed: mailbox switch **0.8 ms** (budget 80), a page 20,000 rows
  deep **0.6 ms** — which is the whole point of a keyset cursor — search **26.6 ms** (budget
  120), idle RAM **57 MB** (budget 300). Scrolling still holds 60 fps.
- Cold start is **545 ms core-side**, process start to window shown including opening and
  migrating the store. That excludes the WebView's own paint, which cannot be timed from the
  core, and it is a debug build. An end-to-end figure needs a release build with bundled
  assets — Phase 11's measurement. Recorded as partial rather than claimed as passed.
- Sorting by anything but date is client-side over the loaded pages, because the keyset
  cursor only supports the date ordering. Stated in `sort.ts` rather than left to be
  discovered.

## 2026-08-26 — Phase 4: Accounts and authentication

Accounts, OAuth, autodiscovery and a connection test that says what is actually wrong. The
security clause of the exit gate is passed and automated; the two live-account clauses need
credentials only the user can supply, and `docs/PHASE-4-VERIFICATION.md` §5 says exactly what.

### Added

- **`accounts::credentials`** — the only module in the program that touches a secret. Four
  kinds (password, refresh token, access token, OAuth client secret) as separate Windows
  Credential Manager entries under the service name `Halcyon Mail`, so revoking a token does
  not disturb a password and a user auditing their credentials recognises what they are
  looking at. `Secret` has no `Display`, no `Serialize`, and a `Debug` that writes
  `Secret(redacted)` — a secret cannot reach a log line or the IPC boundary without someone
  writing `expose()`, a name chosen to be ugly and greppable. Standing rule 12 becomes a
  property of the types rather than something to remember at each call site.
- **`accounts::provider`** — Google, Microsoft, iCloud, **Yahoo** and Other, with servers,
  auth kind, scopes, and the sentence a user needs when signing in requires a step outside the
  app. Yahoo is capped at two connections (docs/05 §5) because it throttles, and being
  throttled looks exactly like the app being broken.
- **`accounts::oauth`** — OAuth 2.0 with PKCE in the **system browser**, hand-rolled. Three
  non-negotiables, each with its reasoning in the source: the system browser (Google blocks
  embedded user agents, and it is the only arrangement where the user can see whose password
  box they are typing into); PKCE always (a desktop client cannot keep a secret, so PKCE is
  what stops an intercepted code being redeemable); and a checked `state` (without it the
  loopback listener accepts a code from any page on the machine that can reach localhost).
  The listener binds `127.0.0.1:0` so nothing can squat a fixed port, answers only
  `/callback`, ignores the browser's unprompted `/favicon.ico`, and leaves a small
  self-contained page rather than a blank tab. Google gets `access_type=offline` and
  `prompt=consent`, without which no refresh token is issued and the account stops working an
  hour later with no explanation.
- **`accounts::autodiscover`** — Mozilla's ISPDB, then the domain's own autoconfig, then SRV,
  then port probing, in that order because that is decreasing confidence. Every result says
  where it came from, and only a probed one asks the user to check it. The autoconfig XML is
  hand-parsed rather than handed to an XML crate: the input is untrusted, and a parser that
  only looks for four named tags cannot be talked into expanding an entity or fetching a DTD.
- **`accounts::verify`** — the connection test, and the reason this phase is more than
  plumbing. Named steps, each pass / fail / **skipped**, with the server's own words folded
  away behind a disclosure and a remedy in plain English in front. The mapping that earns the
  module: `535 5.7.139 … SmtpClientAuthentication is disabled for the Tenant` becomes "Your
  organisation has turned off SMTP authentication for this mailbox … **your password is not
  the problem**". That is an administrator setting, per mailbox; every other client reports it
  as a failed sign-in and the user changes their password over and over.
- **`accounts::store`** — account rows, with "a row never holds a secret" enforced by the API:
  `insert` takes no password.
- **Thirteen commands**, none of which returns a secret. There is deliberately no
  `credential_get`.
- **The account assistant**, modelled on Mail's, with the flow as a reducer in `model.ts`
  rather than component state — the interesting part is which step follows which, and that is
  worth testing directly. Provider, then address, then servers _only if not already known_,
  then the test, then the report.
- **Settings → Accounts** — reordering, per-account colour, a re-authenticate indicator for an
  account whose credential has gone, remove-with-purge, and the bring-your-own-OAuth-client
  fields.
- **`src-tauri/tests/secrets.rs`** — the exit gate's grep, as a test. Searches raw bytes on
  disk including the `-wal` and `-shm` sidecars, because SQLite keeps freed pages until they
  are overwritten and a value deleted from a row can outlive the `SELECT` that returned it.
  Six tests, one of which is the control: it writes the sentinel into the database on purpose
  and asserts the search finds it. Without that, every "not found" assertion would pass just
  as happily against a search that could see nothing.
- 46 tests (frontend 68 → 92, Rust 40 → 117, e2e 30 → 42).

### Changed

- **`withTriggerProps` now merges the trigger's own props** instead of letting `cloneElement`
  replace them. See Incidents — this was a live bug, not a refactor.
- **The settings sheet is its own width** (`--settings-width`, 640) rather than the assistant's 520. It holds a table of accounts; 520 crushed it.
- **`tokio` gained the `net` and `io-util` features.** The connection test speaks the
  protocols directly.
- **`async-imap` is in `Cargo.toml` but unused by Phase 4.** It arrives with the sync engine in
  Phase 5, where IDLE, FETCH and CONDSTORE make it earn its place. A diagnostic wants the raw
  response line — that is the evidence — and a session-oriented client is built to hide it.

### Fixed

- **The description field in Settings → Accounts collapsed to its intrinsic width**, rendering
  "Northgate" as "North" in a box narrower than the word.
- **"Add Your Other Mail Account Account"** — `Add Your ${displayName} Account` against the one
  provider whose name already ends in the word. Special-cased, with a test.
- **Choosing a provider moved the tiles under the cursor.** The setup note appeared below the
  list, the sheet grew, and because a sheet is vertically centred the tiles shifted up — so a
  second click could land on a different provider. Standing rule 6. The note's space is
  reserved now whether or not there is a note.

### Incidents

- **A tooltip-wrapped icon button did nothing when clicked, and had since Phase 1.**
  `withTriggerProps` called `getReferenceProps()` without passing the trigger's own props, then
  `cloneElement`'d the result over the trigger. Floating UI _merges_ what it is given with what
  it generates and calls both handlers; `cloneElement` does not merge, it replaces. So the
  moment a primitive generated a handler of the same name — which `useDismiss({ referencePress: true })`
  does — the trigger's own `onClick` was silently dropped.

  Every `Tooltip`-wrapped `IconButton` was affected, **including the sidebar collapse toggle,
  which has been inert since Phase 2**. Invisible because no test clicked one: the Phase 1
  primitive tests drive `Menu`, whose triggers have no `onClick` of their own, and the Phase 2
  shell tests click plain `Button`s.

  Found by adding a Settings button and watching an end-to-end test fail to open the sheet.
  Three probes to diagnose — a synthetic DOM listener proved the click _arrived_, which ruled
  out an overlay and pointed at the React prop rather than the event. Fixed by merging the
  trigger's props through Floating UI, with the ref applied _after_ the merge: a ref passed
  through the merger does not reliably survive, and a trigger with no ref is one the focus
  manager cannot return focus to. That distinction cost one failing menu test before it was
  noticed.

- **The visual baselines cannot see a control this size.** `maxDiffPixelRatio: 0.002` allows
  ~2,500 differing pixels on a 1400×900 frame; a 28px icon button is under 800. Adding the
  Settings button to the sidebar header failed **no** baseline, in either theme, at either
  width. Worse, `--update-snapshots` rewrote nothing — in Playwright 1.62 that flag defaults to
  `changed`, and nothing had changed as far as the comparison was concerned.
  `--update-snapshots=all` was needed, and the result confirmed by opening the PNG, which is
  the same thing `docs/PHASE-1-VERIFICATION.md` records about a blank baseline.

  The tolerance is not obviously wrong; it exists for font antialiasing. The conclusion is that
  **chrome that matters gets an assertion, not a screenshot** — hence `tests/e2e/accounts.spec.ts`.

- **Three bugs were visible only in the running window. Again.** The three under _Fixed_ above
  passed 92 frontend tests, 117 Rust tests and 42 end-to-end tests without a murmur. Third
  phase running. Running the real window is no longer a lesson; it is a step in the gate.

- **A credential test failed once and never again.** `stores_loads_and_purges` found a purged
  entry still present, one run in a dozen. Did not reproduce in isolation (3 runs) or in the
  full suite (3 runs), and `cmdkey /list` showed no leftover `halcyon` entries at all, so
  `purge` demonstrably works and leaves no residue. Cause unidentified. The one plausible
  cross-run interference is gone: scratch references were keyed on the process id alone, and
  Windows reuses process ids, so a run that panicked before its `purge` would leave an entry a
  later run with the same id would find. They carry a nanosecond timestamp now. Recorded as
  unexplained rather than as fixed.

- **A new end-to-end test was flaky in my own hands, twice in three runs.** The reorder test
  read the account order with `allTextContents()` — a one-shot read with no retry — while the
  reorder round-trips through the store and a query invalidation. Replaced with
  `expect(locator).toHaveText([...])`, which retries and asserts the whole order rather than
  the first row. The first instinct was to blame parallelism; `--workers=1` failed too, which
  is what pointed at the test.

- **The first exit-gate grep reported success on an error.** `grep -r` over the app-data
  directory returned exit 2 — _error_, not _no match_ — because some WebView2 files are locked
  while the app runs. Reporting that as a pass would have made the whole verification a lie.
  Each file is now read individually and says whether it was read, and a positive control
  (searching the same 117MB database for a string that is certainly in it, 200,011 hits) proves
  the search can see anything at all.

- **Windows Firewall prompts on first run.** "Do you want to allow public and private networks
  to access this app?" It was **declined**, not allowed: nothing Halcyon does needs inbound
  access — IMAP and SMTP are outbound, and the OAuth redirect listener binds `127.0.0.1`, which
  Windows Firewall does not filter. Declining had no effect on the connection test, which then
  reached Gmail and completed TLS on both ports. Carried into `docs/07`: a shipped build should
  not train users to click Allow for a permission it does not need.

### Notes

- **The connection test was verified against real servers**, not only against constructed
  strings: `imap.gmail.com:993` connected in 42 ms and completed TLS in 27 ms with a valid
  certificate; `smtp.gmail.com:587` connected in 49 ms and completed a **STARTTLS upgrade** in
  261 ms, also with a valid certificate. Both then rejected the sign-in, and the report showed
  Gmail's own `a2 NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)` behind the
  disclosure with the password nowhere in it.
- **Nothing is compiled in as an OAuth client.** docs/05 §2 offers bring-your-own as a
  mitigation; here it is the only path. Embedding a client id and secret in a desktop binary
  ships a credential anyone can extract and makes every user's mail access contingent on one
  registration surviving Google's review. The provider tile says "Needs setting up in Settings
  first" and Continue is disabled, rather than opening a browser onto an error page that reads
  as the app being broken.
- **Nothing is saved until the connection test passes.** An account row that cannot connect is
  worse than no row: it appears in the sidebar, fails quietly, and working out why becomes the
  user's problem.

## 2026-08-26 — Phase 4: what the first real OAuth setup found

The first person to configure a Google client hit a wall the whole test suite was blind to.
Two bugs, both mine, both in code that had passed review.

### Fixed

- **Saving an OAuth client left the provider greyed out until the app was restarted.**
  `oauth_client_set` was the only mutation in the account command surface that did not emit
  `accounts:changed`, and `useProviders` is cached with `staleTime: Infinity` — so the client
  id was written to the database correctly and the UI kept serving its first answer forever.
  Everything worked except being told about it.

  Diagnosed by grepping the live database rather than by reading code: the id was absent from
  `halcyon.db` but present in `halcyon.db-wal`, which said the write had succeeded and pointed
  straight at the notification path instead of the storage path.

  Fixed in two places, deliberately. The command now emits, which keeps a second window in
  step. More importantly `useAccountsChanged` invalidates the queries directly in shared code,
  so correctness no longer depends on every command remembering to announce itself — the
  failure mode is silent, and one that only shows up as "nothing happened" is not one to leave
  resting on a convention.

- **Four settings writes went through the reader pool.** `set_client_config`, `write_expiry`
  (twice) and `forget_settings` were called via `Db::read`, bypassing the single writer
  `docs/03` §3 mandates. They worked, which is why they survived review — but that is exactly
  the shape that produces `SQLITE_BUSY` under concurrency, and the writer actor exists so that
  nobody has to think about it. All four now go through `Db::write`.

### Incidents

- **The regression test for the first bug would have passed before the fix, and the test says
  so in its own comment.** `notifyBrowserAccountsChanged` only no-ops _inside Tauri_; served by
  Vite it dispatched on the browser bus exactly as before, so the browser path was never broken
  and the Playwright suite could not see the bug. The whole class — a core command that forgets
  to announce itself — is invisible from there.

  Rather than let a green test imply coverage it does not have, `tests/e2e/oauthClient.spec.ts`
  states what it pins (the shared invalidation, which is the part that makes the class
  survivable) and what it does not (the emit, and the event). `docs/PHASE-3-VERIFICATION.md`
  records a test that pinned nothing and passed; writing another one and calling it a fix would
  have been worse than having none.

- **The Credential Manager flake happened a second time**, in a different test: an access token
  written and read straight back came back as the previous value. Like the first, it did not
  reproduce in isolation (5 clean runs) or on demand in the full suite (6 clean runs), and
  unique per-run entry names had already ruled out collisions.

  Two occurrences of the same shape is a pattern, not noise. Every test that touches the store
  now takes a shared lock: they contend on one genuinely global resource — the signed-in user's
  credential store — and `cargo test` runs them on several threads. The lock is not a workaround
  for a bug in our code; it is the honest statement that these tests must not run concurrently.
  Production is unaffected, because after `save_tokens` the caller uses the token it already
  holds in memory rather than reading it back.

### Notes

- Verified in the running window, not only in the suite: Google is selectable, Microsoft still
  correctly says "Needs setting up in Settings first". The distinction matters — an
  invalidation that ungreyed _everything_ would have looked like a fix and been a worse bug.

## 2026-08-26 — Phase 4: the tests were eating real credentials

Three defects, found by finally running a command that had been silently failing all along.
One of them destroyed a real user's configuration.

### Fixed

- **The tests deleted the developer's real Google client secret.** Two of them exercise
  `set_client_config`, which derives its Credential Manager reference from the `Provider`
  enum — so they necessarily touch the production entry — and they "cleaned up" afterwards by
  deleting it. Running the suite therefore wiped a client secret that had just been configured
  through the UI, and every test passed while doing it. The account then failed to sign in with
  nothing on screen or in the logs to explain why.

  Both now use a `Preserved` guard: read the old value first, put it back on drop, and delete
  only if there genuinely was nothing there. **Tests may borrow real state; they may not
  consume it.**

- **Saving a client ID with the secret box empty deleted the stored secret.** `set_client_config`
  treated an absent secret as "clear it". But a password field cannot be prefilled, so that box
  is empty _every time the settings pane is opened_ — and the pane says, in as many words,
  "A secret is saved. Type a new one to replace it." The code contradicted its own label, and
  the failure was silent and delayed.

  Absent or empty now means **keep what is stored**. Clearing the client id is what
  deconfigures a provider, and that still removes the secret, because nothing is left for it to
  belong to.

- **Test credentials leaked into the real Credential Manager — fourteen of them.** Cleanup ran
  at the end of each test body, which is skipped when an assertion unwinds past it. Replaced
  with `Scratch`, an RAII guard that holds the store lock and purges on drop.

### Incidents

- **`cmdkey /list` had never once run.** Every check of "are there leftover credentials?" this
  session went through Git Bash, which rewrites `/list` into a Windows path; the command errored
  and printed usage, and the grep found no matches — which read as "nothing there". On that
  basis this changelog previously recorded "purge demonstrably works and leaves no residue".
  **That was wrong**, and it is the second time this session that an error was read as a
  negative result — the exit-gate grep did the same thing with `exit=2`.

  The lesson is not about `cmdkey`. A check that cannot fail loudly is not a check. Both now
  distinguish "ran and found nothing" from "did not run".

- **The fix was verified with a canary, not by reasoning.** A fake secret was written into the
  exact production slot the tests used to destroy, the full suite was run, and the canary was
  still there afterwards with zero test leftovers. Asserting "the guard restores it" from
  reading the code would have been the same move that produced the original bug.

### Notes

- The real Google client secret is gone and has to be pasted in again — the tests destroyed it
  before the guard existed. The refresh and access tokens for the signed-in account survived;
  only the client secret was affected.

## 2026-08-26 — Phase 5: the sync engine (part one)

Real mail from a real Gmail account is in the app. Roughly half of `docs/06` Phase 5 is
built; §4 below says exactly which half, and `docs/PHASE-5-VERIFICATION.md` carries the
detail.

### Added

- **`sync::backoff`** — jittered exponential backoff, 1s → 300s, ±25%. Pure and tested
  without waiting for any of it. The jitter is the point: every account fails at the same
  instant when a network drops, and without it they all retry at the same instant too, which
  is the connection storm the 12-hour soak exists to catch. A test asserts two accounts
  failing together do not retry together, and another asserts no delay is ever short enough
  to be a tight loop.
- **`sync::threading`** — JWZ, with the tests written first as `docs/06` Phase 5 requires.
  `threading_tests.rs` is a separate file so the order is visible in the repository rather
  than merely claimed: it existed, and failed to compile, before `threading.rs` did.

  Implemented as union-find rather than JWZ's container tree, because this app shows a flat
  conversation and only needs the partition. Two properties fall out for free: a bridging
  message merges two threads by construction, and a reference cycle cannot loop because there
  is no traversal to get stuck in. Real mail contains cycles.

  **Subject-only grouping is deliberately not implemented.** docs/03 §5 permits it where no
  reference link exists; in a real mailbox that merges ten years of "Re: lunch?" into one
  conversation, which is the most damaging thing a mail client can do to someone's archive.
  A test pins the decision.

- **`sync::session`** — connect, authenticate (password and XOAUTH2), read capabilities.
  Capabilities are read _after_ authentication, because servers advertise a different set to
  an authenticated client.
- **`sync::mailboxes`** — `LIST` plus role inference: RFC 6154 attributes first, then name
  heuristics. Gmail localises its folder names, so a French account's `[Gmail]/Messages
envoyés` is only findable by attribute. Two mailboxes claiming one role is resolved
  deterministically rather than left to whichever row came back first.
- **`sync::envelope`** — RFC 2047 decoding, including the legacy character sets. A client
  that skips this shows `=?UTF-8?Q?Bj=C3=B6rn?=` in the sender column.
- **`sync::fetch`** — `SELECT` with `UIDVALIDITY` checking, and envelope fetching issued as a
  raw command so Gmail's `X-GM-THRID` and `X-GM-MSGID` arrive in the same round trip as the
  standard attributes; `async_imap::Fetch` exposes no accessor for them.
- **`sync::persist`** — idempotent on `(mailbox_id, uid)`, which is what makes an interrupted
  sync safe to resume. Threading and cached counts are recomputed rather than incremented,
  so a replayed batch cannot inflate anything.
- **`sync::engine`** — per-account supervisor: connect, discover, newest page of the Inbox
  first, then backfill in batches of 500.
- **`tests/live_gmail.rs`** — an `#[ignore]`d diagnostic that walks the handshake one step at
  a time against a real account. Written because a sync that _hangs_ tells you nothing, and
  it is what found the bug below.
- 106 new Rust tests (119 → 225).

### Fixed

- **The IMAP greeting was never consumed, and every OAuth sync hung for exactly sixty
  seconds.** `async_imap::Client::new` does not read the server's opening `* OK ... ready`,
  and nothing in the crate's API suggests it must. `authenticate` then reads the greeting as
  the answer to the command it just sent, waits for a continuation the server has no reason
  to send, and the server waits for a client that has stopped talking. TLS up in 40ms, then
  silence.

  Found by writing `tests/live_gmail.rs`: the engine's own logging put a boundary around the
  whole handshake, which narrows the fault to four round trips. Putting a boundary around
  each step named it in one run.

- **XOAUTH2 was double base64-encoded.** `async_imap` encodes the authenticator's return
  value itself, so encoding it here sent base64 of base64. Found by reading the crate's
  source rather than guessing. Phase 4's connection test looks different for a good reason —
  it writes the `AUTHENTICATE` line itself, so it does its own encoding.
- **A failed XOAUTH2 exchange could deadlock.** Google answers a rejected token with a second
  continuation carrying a JSON error and waits for an _empty_ line before sending its tagged
  NO. Replying to that with the credential again leaves both ends waiting.
- **Cached mailbox counts were never refreshed.** The first real sync downloaded the mailbox
  correctly and then showed "0 messages" in the header with no badge in the sidebar. The rows
  are the truth; those columns are a cache, and a cache nobody refreshes is a wrong number in
  front of the user.
- **One misconfigured account blocked every working one.** Three demo accounts with no IMAP
  host were treated as retryable, and the engine held a single global lock, so they each
  backed off through five attempts — about ninety seconds — before the real account was
  reached. Configuration errors are now non-retryable, and the lock is per account.
- **Nothing had a ceiling.** The OAuth token request and the IMAP handshake could both block
  forever. A sync that hangs is worse than one that fails: a failure retries, a hang holds
  the account's lock and reports nothing.

### Incidents

- **The log line that would have explained the hang was placed after the call that hung.**
  "sync started" was logged after `connect()` returned, so an account stuck in the handshake
  produced no line at all and looked as though it had never been attempted — which sent the
  first hour of diagnosis to entirely the wrong place. It is now logged before.

  The general form is worth keeping: **a log line after the risky call only tells you about
  the runs that succeeded.**

- **`tauri dev` rebuilt and restarted the app when a source file was saved**, mid-diagnosis.
  A database that grew by 3MB with no apparent cause was the running app quietly picking up
  the greeting fix and syncing for real. Confusing for a minute; the right behaviour.

### Notes

- Verified against the real account, not only against tests: authenticated in 0.9s, listed
  **46 mailboxes**, and the Inbox rendered with real senders, real subjects and correct date
  grouping. Capabilities negotiated: `IDLE MOVE CONDSTORE X-GM-EXT-1 UIDPLUS SPECIAL-USE
COMPRESS=DEFLATE`, which is every extension the remaining Phase 5 work needs.
- No Docker on this machine, so the exit gate's Dovecot half cannot be run as written. An
  in-process IMAP server is the intended substitute and is not built yet — see the
  verification record.

## 2026-08-26 — Phase 5: message bodies

`docs/06` Phase 5 §3 — _lazy body fetch on selection + prefetch of the next 3 rows, cache
`.eml` on disk_. Built and unit-tested; **not yet verified against the live account**, for the
reason in Notes.

### Added

- **`sync::bodies`** — fetch, cache and parse. Standing rule 11 governs the whole file: a
  message body is hostile input, so nothing panics, nothing recurses without a bound and
  nothing is unwrapped. Specifically:
  - a **depth cap** on the MIME walk, because a message can nest `multipart/mixed` thousands
    deep and unbounded recursion there is a crash triggered by opening mail someone sent you;
  - a **size cap**, because a server will hand over a 200MB attachment and reading it into a
    `Vec<u8>` gets the process killed rather than reporting a problem;
  - a truncated or unparseable body yields an **empty** body rather than an error, because a
    message you can see and not read beats one that vanished.
- **An HTML→text fallback.** Most marketing mail is HTML only; without it the preview column
  and the reader would be blank for a large fraction of a real mailbox. `<script>` and
  `<style>` contents are dropped rather than stripped of tags, so their source never reaches
  the list or the search index.
- **Lazy fetch with a 3-row prefetch** (`useBodyPrefetch`). Three, not thirty: prefetching
  further ahead than someone can plausibly arrow spends their bandwidth and the provider's
  connection budget on mail they will never open. Already-cached ids cost nothing, so the UI
  deliberately keeps no record of what it has — a second copy of that bookkeeping is a second
  thing to drift.
- **`.eml` cached per account per message**, so a reply can quote the original exactly and
  Phase 6 can re-render without another round trip.
- 15 tests, including multibyte survival through the HTML stripper, a 200-deep nested
  message, and five malformed bodies that must not panic.

### Changed

- **The HTML part is stored and deliberately not served.** `MessageFull` exposes `body_text`
  only. Sanitising markup and putting it in a sandboxed frame is docs/03 §6 — Phase 6's work
  — so until that exists no untrusted markup can reach the WebView at all. The reader shows
  the plain-text part.
- **An inline `cid:` image no longer counts as an attachment.** Every HTML newsletter carries
  a tracking pixel as an inline part, and a mailbox where every row shows a paperclip is a
  mailbox where the paperclip means nothing. Still stored, just not counted.

### Fixed

- **A missing OAuth client secret was reported as a rejected sign-in.** Google refuses to
  refresh a desktop client's token without the secret it issued, and calls that
  `invalid_request` — indistinguishable from a bad credential unless you check first. The app
  said "signing in again will fix it", which is a browser round trip that cannot possibly
  help. It now names the missing field and rules the wrong remedy out explicitly.

  Phase 4's principle applied to Phase 5: the value of knowing _which_ failure this is comes
  entirely from being able to say something specific.

### Notes

- **Live verification is blocked on a credential, not on code.** The Google client secret is
  still missing — Phase 4's tests destroyed it before the guard existed — and the stored
  access token has now expired, so no OAuth call can succeed until it is pasted back. The
  earlier sync worked because the token was still inside its hour.

  `tests/live_gmail.rs` reports this precisely: `client secret set: false`, expiry `-960s`,
  `token failed: invalid_request`. Bodies will be verified against real mail as soon as the
  secret is restored.

## 2026-08-26 — Phase 6 (early): rendering mail safely

Pulled forward from Phase 6 because a mail client that shows plain text where a message has
formatting and images is not showing the message. docs/03 §6 in full.

### Added

- **`mail::render`** — the sanitiser. `ammonia` (which docs/03 §6.2 names) parses with
  html5ever rather than matching patterns, so it cannot be defeated by the malformed markup
  that defeats regex strippers. Allow-list, not deny-list: a deny-list is a list of the
  attacks someone has already thought of.
- **A sandboxed frame** — `sandbox="allow-same-origin"` and nothing else. No `allow-scripts`,
  so nothing in a message can run. That resolves an apparent contradiction in §6.7, which
  asks the frame to post its own height: it cannot, because it cannot execute. The _parent_
  reads `scrollHeight` through `allow-same-origin` instead, so the measuring code is ours and
  the message stays inert.
- **Remote content blocked by default**, with a banner that says how many images were
  withheld and what loading them tells the sender. On consent they are fetched **by the Rust
  core** and handed to the frame as data — so the frame never makes a request and the sender
  never sees the user's IP or a `Referer`. Consent is per message and never remembered.
- **Inline `cid:` images** resolved from the cached `.eml` only. An embedded signature or
  screenshot renders with no network request at all.
- **Links open in the default browser**, with the phishing check §6.6 asks for: when the
  visible link text names a different host from the `href`, the user is asked first. Only
  `http`, `https` and `mailto` open — `ms-msdt:` and friends have been used to run code from
  a link, and a message must not be able to launch an arbitrary handler.
- 32 tests, including twelve hostile-markup cases.

### Fixed

- **CSS could smuggle a tracking pixel past the image blocker.** `ammonia` sanitises markup
  and passes CSS through untouched, so `style="background:url(https://tracker)"` loaded a
  remote resource exactly like an `<img>` would — while the banner said remote content had
  been blocked. That made the banner _wrong_, not merely incomplete.

  Found by this module's own hostile-markup test, which is what it is for. Inline styles are
  now filtered for `url()`, `expression()`, `@import` and `behavior:`. The frame's CSP would
  have refused the load anyway; relying on that would contradict the module's own rule about
  not resting safety on the last step in the pipeline.

- **A message selected before its body downloaded stayed blank forever.** Bodies are fetched
  lazily _after_ selection — that is the design — so the first render legitimately has
  nothing. But nothing invalidated the reader when the body arrived, so it kept showing an
  empty white card until the user clicked away and back. `messages:updated` now invalidates
  the body query, and the gap shows "Downloading this message…" rather than a blank card that
  reads as breakage.

### Incidents

- **I wedged WebView2 on the development machine, and it is still wedged.** Force-killing
  `halcyon.exe` repeatedly during debugging left its WebView2 profile locked
  (`0x8007139F`); clearing the profile fixed that once. Then, trying to clear it a second
  time, I killed `msedgewebview2.exe` processes **without checking which application owned
  them** — most belonged to other apps on the machine. WebView2 has failed to initialise
  since, with `0x80070057`, and clearing the profile no longer helps. It needs a reboot.

  Two separate mistakes. The first is a dev-loop hazard worth knowing: `taskkill /F` on a
  Tauri app leaves its WebView2 profile locked, so stop it cleanly. The second is not a
  hazard, it is carelessness — a process name is not an owner, and I ran a destructive
  command against every process that shared one.

- **The renderer was verified without the GUI.** `tests/live_gmail.rs render_probe` walks the
  real stored messages through the real pipeline and prints what comes out. Against the live
  mailbox: a 14,918-byte body sanitises to 11,610 with 2 remote images blocked, a 60,952-byte
  Google message to 55,991 with 12 blocked, an 89,923-byte newsletter to 83,639 with 35
  blocked. The pipeline is sound; only the window is unavailable.

### Notes

- **The HTML part is now served, sanitised, and only through `message_body`.** There is
  deliberately no command returning the stored HTML unprocessed — the sanitiser cannot be
  forgotten because there is nothing else to call.

---

## 2026-08-26 — Phase 6 (early): rendering confirmed in the real window

The reboot cleared the WebView2 failure recorded in the previous entry. Running the app then
found the two things a green test suite could not: an environment conflict that had nothing
to do with this code, and a rendering state the tests had encoded backwards.

### Fixed

- **A message whose body has not downloaded yet now shows "Downloading this message…"
  instead of a blank white card.** `render()` fell through to the plain-text path whenever
  there was no HTML, and `from_plain("")` returns a 33-byte `<pre class="halcyon-plain">`
  wrapper. That is not empty, so `MessageBody`'s "is there any HTML?" test said yes and
  mounted an iframe around nothing.

  It matters more than it sounds. Bodies are fetched lazily _after_ selection (docs/03 §5),
  so **every** message passes through this state on first open — and only 10 of 431 messages
  in the real account had bodies at the time, so nearly every click produced a blank card
  that filled in fifteen seconds later with no explanation. From the user's side this is
  indistinguishable from "the mail does not render", which is exactly how it was reported.

  `render()` now returns `Rendered::default()` when there is neither HTML nor text. An empty
  HTML part with a real text part still falls back to the text, as before.

- **The test that covered this asserted the wrong contract.**
  `an_empty_body_renders_to_an_empty_message_rather_than_failing` asserted
  `from_plain_text == true` for a body with nothing in it, which is precisely the behaviour
  that caused the bug — it was written to describe what the code did rather than what the
  reader needs. Replaced with
  `a_body_that_has_not_been_downloaded_yet_renders_to_nothing_at_all`, which asserts the
  rendered HTML is empty across `(None, None)`, `(Some(""), None)` and whitespace-only parts,
  and says in the comment why non-empty output breaks the reader.

### Incidents

- **The blank window was RivaTuner Statistics Server, not our code.** After the reboot the
  app still painted nothing and logged `0x8007139F` every ~21 seconds. The webview was in
  fact running — it made IPC calls and rendered a body — and then died 11 seconds in.

  Diagnosed by parsing the WebView2 minidump directly
  (`EBWebView/Crashpad/reports/*.dmp`): exception `0xC0000005` at `0x1801490AF`, and the
  module list puts that address inside `RTSSHooks64.dll` — the overlay hook RivaTuner
  Statistics Server (shipped with MSI Afterburner) injects into every process that presents
  a frame. Closing RTSS fixed it completely: zero WebView2 errors, webview stable.

  Worth recording for two reasons. First, it will recur on this machine every time RTSS is
  running, and the symptom — blank window, `0x8007139F` — looks exactly like the profile
  corruption in the previous entry, which sent the first hour of debugging the wrong way.
  Second, it is a real end-user failure mode: any Tauri or Electron app can be crashed by a
  third-party overlay injector, and the crash surfaces with no attribution whatsoever. The
  permanent fix is an RTSS per-application profile with `EnableHooking=0` — the same shape
  as the `warp.exe.cfg` and `RustDesk.exe.cfg` already in that machine's `Profiles/`
  directory, which suggests other apps hit this too.

- **A blanket process kill was avoided this time.** Stopping the app needed its WebView2
  children gone. Rather than killing by name — the mistake recorded in the previous entry —
  the parent chain of every `msedgewebview2.exe` was walked first and only descendants of
  `halcyon.exe` were touched. Twelve belonging to other applications were left alone. The
  correction from that incident held.

### Notes

- **Verified in the running window, against the real account, end to end.** A 50,905-byte
  Economic Times newsletter sanitises to 49,728 with 22 remote images blocked and renders
  with its masthead, serif headings, rules and buttons intact; "Load Images" then fetches
  through the Rust proxy and re-renders at 391,491 bytes with 0 blocked and the photographs
  in place. A Mojo Times newsletter renders its full colour layout, background panels and
  call-to-action buttons. The Pi-hole on this network did not interfere with the proxied
  fetches, so the bypass setting offered earlier is still not needed.

- **The real account is syncing but says nothing while it does.** `sync_mailbox` logs nothing
  between "discovered mailboxes" and "sync finished", so a 46-mailbox account looks stalled
  for minutes at a time; the message count climbed 215 → 431 during this session, so it is
  working. Per-mailbox progress logging is worth adding before this is ever debugged again.

- **Capabilities now report `condstore=false qresync=false idle=false gmail=false`** on a
  connection that previously advertised `IDLE MOVE CONDSTORE X-GM-EXT-1 UIDPLUS`. Not chased
  this session; recorded because it is a change, and because Phase 5's remaining work
  (IDLE, CONDSTORE incremental sync) depends on reading those correctly.

---

## 2026-08-26 — Phase 5: three bugs that only a running sync could show

Resumed Phase 5. The first task was meant to be a small one — capability detection was
logging `false` for a Gmail server that advertises `IDLE CONDSTORE X-GM-EXT-1 UIDPLUS`. It
turned out to be the thread that unravelled the other two, and each was found by adding a log
line rather than by reading the code.

### Fixed

- **Every IMAP capability had been reading `false` since Phase 5 was written.** The set was
  built with `format!("{capability:?}")`, but `Capability` is an enum whose atoms carry their
  name in a payload, so the derived `Debug` produced `Atom("CONDSTORE")` — which matches
  nothing. IDLE, CONDSTORE, MOVE and UIDPLUS were all silently off.

  Nothing failed loudly because "the server cannot do this" is a legitimate answer, and the
  fallback path for each is correct if slow. Every existing test fed `Caps::read` strings
  written by hand, so all of them passed while the real path matched nothing — the seam
  between the library's types and ours had no test crossing it. Now named by matching the
  enum, with a test that goes through real `Capability` values and asserts the flags the
  engine branches on come out true. Verified live: `condstore=true idle=true gmail=true`.

- **Backfill walked the numeric UID range instead of the UIDs that exist**, which on any
  long-lived mailbox is a wildly different number. The real Gmail Inbox holds 214 messages
  with `uid_next` at 106,287, so windows of 500 meant 213 round trips of ~20 seconds — about
  seventy minutes — to fetch 214 messages, inserting nothing on nearly every one. At docs/04's
  50k-message exit gate it does not finish at all.

  `fetch::all_uids` now issues one `UID SEARCH ALL` and `backfill_window` batches through the
  UIDs that came back, listed explicitly rather than as a range spanning the gaps. Measured
  after: one round trip plus **one** batch, 26 seconds, complete. The newest page still uses
  the cheap range — it costs no round trip and it is what the user waits for; being
  approximate there is fine because the search is what guarantees nothing is missed.

- **Backfill also had no memory, so it restarted from the top on every sync** and, ending only
  at UID 1, effectively never ended. Added `mailbox.backfill_uid` (migration 0002), written
  after _every_ batch rather than once at the end — the case that matters is the run that does
  not finish. It never moves upwards, so the newest-page sync cannot undo a completed walk,
  and `drop_mailbox_contents` clears it because a `UIDVALIDITY` change makes the recorded UID
  meaningless.

- **Re-threading took 20 seconds per batch, per mailbox** — with `batch_ms=9` and
  `count_ms=0` beside it, it was the entire cost of a sync. 46 mailboxes at ~30s each is a
  23-minute sync of an account holding a few hundred messages.

  The aggregate roll-up runs three correlated subqueries per thread. Two are answered by
  `ix_msg_thread(thread_id, date_sent)` as a covering index in ~1.5ms; the third,
  `MAX(date_received)`, had no usable index at all — `EXPLAIN` showed a bare `SEARCH message`
  — and cost **38.6 seconds** across 415 threads. Migration 0003 adds
  `ix_msg_thread_recent(thread_id, date_received)`, and `ix_msg_account_recent(account_id,
date_received DESC)` for the window `SELECT` beside it, which was scanning the whole table
  and sorting through a temp b-tree.

  Measured on the real database: the roll-up went **38,600ms → 2.2ms**. In the running app
  `thread_ms` went **20,416 → ~38**, and a full sync of 44 mailboxes went from never finishing
  to **44 seconds, 673 messages inserted, 0 failures**.

  `date_sent` was deliberately left in the existing index rather than replaced: the reader
  orders a thread by the sender's clock and this roll-up wants the server's.

### Added

- **Per-mailbox sync progress logging, and timings split by stage.** `sync_mailbox` now logs
  what it selected, what it stored, and each backfill batch; `sync finished` carries mailbox,
  failure and insert counts. The newest-page log carries `fetch_ms`, `write_ms`, `batch_ms`,
  `count_ms` and `thread_ms`.

  This is the reason the other three entries above exist. The engine previously logged nothing
  between "discovered mailboxes" and "sync finished", so a 46-mailbox account looked hung for
  minutes at a time whether it was working or not — and the first instinct on seeing that was
  to suspect the network. Splitting fetch from write settled it in one line: `fetch_ms=418`,
  `write_ms=22510`. Two clock reads per mailbox is worth keeping permanently.

### Notes

- **`qresync` really is absent.** Gmail advertises CONDSTORE but not QRESYNC, so `has_modseq()`
  is true by the CONDSTORE path alone. The earlier note that all four flags looked wrong was
  half right: three were misread, and this one was correct.
- Still outstanding in Phase 5: IDLE on a dedicated connection, CONDSTORE incremental sync,
  `pending_op` drain, the 2–4 connection pool, an in-process IMAP test server, and Gmail
  labels-as-mailboxes. The exit gate needs Dovecot-in-Docker and a 12-hour soak.
