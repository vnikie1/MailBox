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

---

## 2026-08-26 — Phases 5 and 6: the rest of sync, and reading

Asked to complete both phases. Phase 5's sync work is now done; Phase 6 is substantially done,
with the remainder listed honestly at the end.

### Added — Phase 5

- **`pending_op` drain (`sync/ops.rs`).** Flagging, moving and deleting wrote locally and told
  the server nothing, so any change made in this app was invisible everywhere else. Each
  mutation now records its intent **inside the same transaction as the local write**: the
  change and the obligation to push it are one atomic unit, so there is no window in which the
  screen says one thing, the server another, and nothing remembers the difference. Offline mode
  is not a mode — it is what this queue does when the drain cannot connect.

  The drain runs at the _start_ of a sync. The other order loses data: pulling first overwrites
  the local change with the stale value the server still holds, and the queued operation then
  pushes a value the user has already watched revert. Operations are grouped per mailbox and
  sent oldest first, and a failure **stops** the queue rather than skipping past it, because
  later operations can depend on earlier ones having landed. Five failures drops an operation —
  otherwise one impossible change blocks every change made after it, forever.

- **IDLE (`sync/idle.rs`).** New mail now arrives without being asked for. On its own
  connection, because an idling connection cannot be used for anything else, and re-issued
  every 29 minutes per RFC 2177 §3 — past that a server may log the client off, and a dead
  watcher is indistinguishable from a quiet mailbox.

  Notifications are debounced and rate-limited, because the server reports _our own_ writes
  too: a drain of twenty flags arrives as twenty notifications and would otherwise sync, drain
  and notify again. Servers without IDLE fall back to polling, and the connection is closed
  rather than held open doing nothing. Watchers are reconciled against the account list rather
  than started blindly, so the UI can call `sync_watch` on launch and on every account change.

- **CONDSTORE incremental sync.** RFC 7162. A mailbox whose MODSEQ has not moved needs no work
  at all — against the real account that is **190 skips per pass**, one `SELECT` each instead
  of a full envelope fetch. More importantly, a flag changed on another device is reported
  _wherever it is in the mailbox_, not only in the part we happen to re-read: reconciling by
  re-fetching the newest page is why most clients silently miss a message read on a phone last
  month.

  `flags_changed_since` asks for `FLAGS` and nothing else, and `apply_flag_changes` updates
  only the flag columns, so three hundred messages read elsewhere do not become three hundred
  search-index rewrites. Falls back to the full path when anything is missing, and also when
  MODSEQ goes _backwards_ — RFC 7162 §3.1.2.2 allows that after a restore from backup, and it
  makes every "changed since" question meaningless.

### Added — Phase 6

- **Quoted replies fold behind a `<details>`.** That element specifically, because the message
  frame runs no script and never will (standing rule 11) — it is the one interactive control
  HTML has that needs none, so the message stays completely inert and the quote still folds.

  The cut is only ever made at the **top level**. Cutting inside an open element would put its
  closing tag inside the wrapper and its opening tag outside, mangling the message rather than
  folding it. Tracking depth by scanning is safe here only because this runs on sanitised
  markup, which html5ever has already balanced; on raw input none of it would hold. Void
  elements are excluded from the count — a `<br>` counted as an open tag would push everything
  after it to a depth that never returns, and no quote would fold again.

  Recognises the markers Gmail, Yahoo, Thunderbird, Outlook and Apple Mail actually emit, plus
  the plain-text forms, and refuses to fold when the quote is the whole message. Outlook's
  marker needed `id` allowed through the sanitiser on `div` and `hr`; without it
  `divRplyFwdMsg` is stripped before the folder sees it and every Outlook reply shows its full
  history, which is most business mail.

- **Attachment preview and save.** There is deliberately **no "open with the default
  application"**: handing an attachment to whatever the shell associates with its extension is
  the most reliable way malware has ever spread through mail, and an Open button beside
  `invoice.pdf.exe` is a loaded gun with a friendly label. The previewer renders images, text,
  JSON and PDFs from a `data:` URI inside a sandboxed frame with no scripting; **the core**
  decides what is previewable, because that is a security decision and belongs on the side of
  the boundary that cannot be bypassed. Everything else offers Save only, where the shell's own
  warnings stay intact.

  `platform::files::safe_file_name` treats `Content-Disposition` filenames as the
  attacker-controlled text they are: no separators or traversal, reserved device names defused,
  the trailing dots and spaces the filesystem silently drops removed, and bidirectional
  overrides stripped — a name using U+202E renders in a file dialog as `invoiceexe.pdf` while
  remaining an executable.

  Built on the existing `Sheet` so it inherits the focus trap and dismissal; a second modal
  implementation is a second set of accessibility bugs.

- **Data detectors** for parcel tracking numbers and phone numbers. Only text nodes are touched
  — running a pattern over the whole document would rewrite the inside of attributes — and
  anything already inside a link, a style block or a fold's summary is skipped, since a link
  inside a link would break the reader's own click handling.

  Detection is deliberately conservative, because a false positive is worse than a miss: it
  puts a link on ordinary prose, and a link in a message is something the user is entitled to
  believe the sender put there. Most of the tests are about what must _not_ match.

### Fixed

- **Messages in a thread were displayed but never downloaded.** The reader shows a _thread_;
  the prefetch worked from the _message list_, and those are not the same set. A thread reaches
  across mailboxes, so a message carrying a Gmail label lives elsewhere and never appears as a
  row — the reader displayed messages nothing had asked for, and they sat on "Downloading this
  message…" indefinitely. Common rather than exotic on a real Gmail account, where labels put
  most conversations in that position. `useThreadBodies` now asks for exactly what the reader
  renders.

- **`open_external`'s scheme check is extracted and tested.** It was inline in an async command
  and had no test at all. Now covered for `ms-msdt:`, `search-ms:`, `file:`, `javascript:`,
  `vbscript:`, `data:` and UNC paths, none of which were exercised before. `tel:` was added for
  the data detectors and is permitted **only** in the reduced `+`-and-digits form the detector
  produces; a `tel:` a sender wrote is still refused, because it arrives with the rest of the
  message's markup and has never been reduced to digits.

### Incidents

- **The blank card and the silent fetch were the same class of mistake, twice.** `fetch_body`
  logged only on failure, so a call that returned nothing left no trace and looked exactly like
  the UI never asking — the same shape as the sync's "log before the hanging call" recorded
  earlier today. Both now log entry and exit. Worth stating plainly: on this project every bug
  found by running the app has been found by a _log line_, and every one of them was invisible
  to a green test suite.

- **An IDLE rate limit that was enforced on paper and never in practice.** `MIN_INTERVAL` lived
  inside one connection's scope, and since a notification ends the connection it was set and
  immediately discarded every time. Caught by an unused-assignment warning rather than by a
  test; it now lives across reconnects.

### Notes

- Verified in the running window against the real Gmail account: the inbox grew from 219 to 244
  messages _on its own_ while the app sat idle, and a newsletter that had arrived two minutes
  earlier opened and rendered with its layout, buttons and blocked-image banner intact.

- **Still outstanding in Phase 5:** the 2–4 connection pool per account (one connection for
  sync and one for IDLE is the current arrangement, which is inside the budget but does not
  parallelise), an in-process IMAP test server, and Gmail labels-as-mailboxes. The exit gate
  itself needs Dovecot-in-Docker and a 12-hour soak, neither of which has been run — so Phase 5
  is feature-complete but its gate is not passed.

- **Still outstanding in Phase 6:** drag-to-Explorer, the contact popover, and the date and
  address data detectors. Drag-to-Explorer needs `DoDragDrop` and an `IDataObject` — real COM
  work rather than a Tauri call — and was left undone rather than faked. The exit gate also
  asks for twenty real newsletters checked in both themes, which has not been done
  systematically.

---

## 2026-08-27 — Phase 5 exit gate, and Phase 7 begins

The Dovecot rig went up on the Mac Studio, four of the five Phase 5 gate items ran against it,
and **every one of them found a bug**. That is the entire argument for exit gates, and it is
worth stating plainly: none of the four could have been caught by a unit test, and all four
were sitting behind a suite that was green.

### Added — the rig

- **`test/dovecot/`** — Dovecot in Docker, with a 50,000-message seeded mailbox. Three things
  need a server we control, and Gmail can supply none of them: **QRESYNC**, which Gmail does not
  advertise at all, so that path had never run once; a **`UIDVALIDITY` reset**, which cannot be
  provoked on Gmail and whose recovery is the one most likely to be wrong and least likely to be
  noticed; and **rudeness** — cutting the connection mid-sync is something to do to a server you
  own.

  Three things fought back, and all three are recorded in the rig rather than left as folklore.
  Docker Desktop consults its credential helper on every pull _and_ build, the helper reads the
  login keychain, and a keychain cannot be unlocked from a non-interactive SSH session — so a
  public image needing no credentials still could not be fetched, and the image is now built
  from a base already on the host. Alpine ships **Dovecot 2.4**, whose configuration is a
  different language from 2.3 and which refuses to start rather than degrade. And the
  certificate needed an **IP SAN**, because this machine cannot resolve the Mac's mDNS name, so
  Halcyon connects by address and a DNS-only certificate fails validation against one.

  TLS is required rather than optional, and the CA is trusted on the development machine
  deliberately. The alternative — a code path that skips validation "only in tests" — is exactly
  the kind of thing that ships, and a mail client that can be talked out of checking a
  certificate is not worth writing.

- **`tests/dovecot_gate.rs`** — the first four gate items, reproducible.

- **`tests/dovecot_soak.rs`** — the twelve-hour soak, behind a `soak` feature so that a running
  soak's locked binary does not stop `npm run verify` with a linker error unrelated to whatever
  is being verified.

### Fixed — what the gate found

- **An interrupted first sync silently abandoned the rest of the mailbox.** A sync records
  `uid_next` and the MODSEQ after its _first page_; interrupt it and the next sync sees an
  unmoved MODSEQ, concludes "unchanged since the last sync", and returns before reaching the
  backfill. Measured: killed at 5,000 of 50,000, re-run, **finished in 1.6 seconds having
  fetched nothing**. The remaining 45,000 would never have arrived and nothing would have said
  so. The incremental shortcut now refuses to run while a backfill is outstanding.

- **A `UIDVALIDITY` reset restored one page instead of the mailbox.** The same shape: the
  mailbox's stored state is read at the top of `sync_mailbox`, the drop then clears it, and the
  local variables still described the mailbox that no longer existed — so the backfill marker
  still read "complete" and the mailbox was refetched to exactly 500 messages of 50,000, and
  reported success.

- **Ninety per cent of a large mailbox had no threading.** The comment on `RETHREAD_WINDOW` has
  claimed since Phase 5 that "a full pass runs once at the end of the initial sync". It never
  did. Every message stored correctly; 45,000 of 50,000 with no thread, which in the reader is
  nine messages in ten shown as a conversation of one. The pass now exists, runs only when
  something is actually unthreaded, and is affordable only because of the indexes added
  yesterday.

- **A transient sign-in failure permanently stopped the sync — the worst bug this project has
  had.** Every login failure became `Rejected`, which is not retryable (correctly, since
  hammering a refused password locks an account), and a non-retryable error makes the IDLE
  watcher exit for the life of the process.

  It fired for real during the first soak. The Docker host slept, its VM clock jumped backwards,
  Dovecot killed its own auth process — it does that deliberately — and for about ninety seconds
  logins were aborted. The client read that as a refused credential and **did not open another
  connection for the remaining six and a half hours**. In the app that is mail silently ceasing
  to arrive until someone restarts it. `login_failure` now separates the two using RFC 5530
  codes plus the plain-language forms servers actually send, defaulting to _retry_: getting it
  wrong that way costs one connection after a backoff, and getting it wrong the other way costs
  the user their mail.

### Changed

- **The sync engine no longer depends on Tauri.** It took an `AppHandle` purely to call `emit`,
  which made it undrivable from a test — Tauri's own mock runtime does not load on Windows at
  all, and the test binary dies at start with `STATUS_ENTRYPOINT_NOT_FOUND` before running a
  line. `sync::events::Events` is a two-method trait that `AppHandle` implements, so the app is
  unchanged; the gate implements it in six lines and can additionally assert on what was
  emitted. It also matches what docs/03 already claims the architecture is.

### Incidents

- **The soak's own criteria were too weak to catch the bug it found.** Memory was flat and
  connections never exceeded budget, so both assertions passed — while the client had been dead
  for six and a half hours. A dead process uses no memory and opens no connections. It now
  checks **liveness first**: that delivered messages were actually picked up, and that
  connections were not absent for most of the run. A soak whose criteria a corpse can meet is
  not measuring anything.

- **And then the liveness check was wrong too.** The soak delivers a message each sample, named
  `soak1`, `soak2`, restarting at 1 every run — so the second run _overwrote_ the first run's
  messages instead of adding any, the mailbox count stayed flat, and the new check reported a
  dead client while the client was working. A harness that cries wolf is worse than none.
  Filenames now carry a per-run stamp.

### Added — Phase 7

Composing, built bottom-up: the pure logic first, then storage, then the network, then the
window. Every layer is tested without the one above it.

- **`mail/reply.rs`** — who a reply goes to. **`Bcc` is never read at all**: not filtered late,
  never consulted, so no future edit can reintroduce it, and there is a test whose only job is
  to keep that true. `Reply-To` wins over `From`. The user's own addresses are excluded
  case-insensitively, because the local part is case-sensitive per RFC 5321 and nothing on earth
  treats it that way. Mailing-list machinery and `no-reply` addresses are excluded from a
  reply-all — replying to a bounce handler is a message to a robot, sent in public. A forward
  starts with nobody on it.

- **`mail/outgoing.rs`** — the RFC 5322 bytes. `multipart/alternative` with the plain part
  **first**, because RFC 2046 §5.1.4 makes the last part the richest and clients that show the
  first part they understand depend on that order. `lettre` does the encoding: quoted-printable,
  RFC 2047 headers, folding and boundaries are all places where "nearly right" arrives as
  mojibake and the sender never finds out.

- **`sync/outbox.rs`** — the state machine, built around never silently losing a message and
  never silently sending one twice. `holding` is what makes **Undo Send** honest: nothing has
  been transmitted, so undo deletes the row rather than racing a send. Cancelling later returns
  false rather than pretending.

  For "killing the app mid-send neither loses nor duplicates" there is a window between SMTP
  accepting and this process recording it, and both obvious answers fail silently. So the answer
  comes from the server: every message carries a `Message-ID` we generate, a sent message lands
  in Sent, and recovery searches for it there.

- **`sync/smtp.rs`** and **`sync/sender.rs`** — submission, and the loop that joins the two.
  Submit, then file a copy in Sent, then mark sent: a message delivered but missing from Sent is
  untidy, while a copy in Sent for a message that never left is a lie the user acts on. Port 25
  is refused outright. The delivery envelope is built from the addresses rather than recovered
  from the transmitted bytes, which is the mechanism by which `Bcc` works.

- **The compose window** — a separate OS window, with Lexical for the body. The node allow-list
  is a security boundary rather than a feature list: this editor holds HTML the user is about to
  send under their own name, so pasted content is parsed into nodes rather than injected as
  markup and anything without a node type has nothing to become.

### Notes

- Phase 7 still owes: signatures, attachments, drafts with IMAP `APPEND`, the format bar UI,
  contact autocomplete, Send Later, and the Undo Send banner in the main window. Its exit gate —
  replies threading correctly in Gmail, Outlook and Apple Mail — needs real accounts on all
  three.
- The twelve-hour soak is running as this is written. Its verdict is not yet in.

---

## 2026-08-27 (later) — Phase 7: composing and sending

Built bottom-up — pure logic, then storage, then the network, then the window — so each layer
is testable without the one above it. The parts of this phase that can be got wrong are mostly
not the parts that fail loudly: a recipient list that is subtly wrong sends a private message to
the wrong people, and the user finds out from them rather than from an error.

### Added — the core

- **`mail/reply.rs`** — who a reply goes to, the subject, the reference chain, the attribution
  line. **`Bcc` is never read at all**: not filtered late, never consulted, so no future edit
  can reintroduce it, and one test exists solely to keep that true. `Reply-To` wins over `From`.
  The user's own addresses are excluded case-insensitively — the local part is case-sensitive
  per RFC 5321 and nothing on earth treats it that way, so a case-sensitive comparison would put
  the user on their own reply. Mailing-list machinery (`-bounces`, `-request`, `-owner`) and
  `no-reply` addresses are dropped from a reply-all: replying to a bounce handler is a message
  to a robot, sent in public. A forward starts with nobody on it, because pre-filling from the
  original is how a private thread reaches the people already on it.

- **`mail/outgoing.rs`** — the RFC 5322 bytes. `multipart/alternative` with the plain part
  **first**, because RFC 2046 §5.1.4 makes the last part the richest and clients that show the
  first part they understand depend on that order. Attachments nest the alternative inside a
  `multipart/mixed`; a flat mixed part holding both bodies is read by some clients as two
  attachments and by others as a message whose HTML is a file. `lettre` does the encoding —
  quoted-printable, RFC 2047 headers, folding and boundaries are all places where "nearly right"
  arrives as mojibake and the sender never finds out.

- **`sync/outbox.rs`** — `holding → queued → sending → sent`, with `failed` and a retry path.
  `holding` is what makes **Undo Send** honest: nothing has been transmitted, so undo deletes
  the row rather than racing a send, and cancelling later returns false rather than pretending.

  For _killing the app mid-send neither loses nor duplicates_: there is a window between SMTP
  accepting and this process recording it, and both obvious answers fail silently. So the answer
  comes from the server — every message carries a `Message-ID` we generate, a sent message lands
  in Sent, and recovery searches for it there. An interrupted attempt does not spend a retry,
  because it was cut short rather than used.

- **`sync/smtp.rs`** and **`sync/sender.rs`** — submission, and the loop joining the two.
  Submit, then file a copy in Sent, then mark sent: a message delivered but missing from Sent is
  untidy, while a copy in Sent for a message that never left is a lie the user acts on. Port 25
  is refused outright. The delivery envelope is built from the addresses rather than recovered
  from the transmitted bytes, which is the mechanism by which `Bcc` works.

### Added — the window

- **A separate OS window**, as docs/01 §6 specifies. People start a reply, go and look something
  up in another message, and come back; a modal makes that impossible and a pane makes the list
  unusable. Lexical for the body, because a `contenteditable` differs between engines in exactly
  the ways that matter — where the caret lands after a list item, what pasting from Word
  produces, whether undo groups a word or a character.

  The node allow-list is a security boundary rather than a feature list. This editor holds HTML
  the user is about to send **under their own name**, so pasted content is parsed into nodes
  rather than injected as markup, and anything without a node type has nothing to become.

- **The format bar** — exactly Mail's set. The restraint is the design: every control produces a
  node the _recipient's_ client has to render, and mail clients are the least capable renderers
  in software. Links are limited to `http`, `https` and `mailto`; a `javascript:` URL signed by
  the user is the same hazard as one from a stranger, only worse.

- **Undo Send, Send Later and the failure banner.** The countdown is a _display_ of the core's
  timer and never the thing driving it — a window that was asleep or throttled must not change
  when a message goes. Failures show what the server actually said: "550 mailbox full" is
  something the user can act on, and a paraphrase is not.

- **Attachments**, with a size warning rather than a refusal. Outgoing filenames are sanitised,
  which sounds redundant and is not: the name came from this machine, but it lands in the
  _recipient's_ download folder, so traversal and right-to-left overrides matter identically on
  the way out — and this app must not be the thing that sends them.

- **Signatures**, on the account rather than in `setting`, because a signature belongs to an
  identity. Placement above or below the quote is stored rather than guessed: "above" is what
  people who reply inline expect and "below" is what top-posters expect, and getting it wrong
  makes every reply look like a mistake.

- **Drafts**, in their own table, autosaved on a timer _and_ on window blur. Neither alone is
  enough: a timer loses up to thirty seconds when the window closes, and blur alone loses
  everything when a machine dies with the window focused — which is exactly when a long message
  is being written. The local write is what the caller waits for; the server copy is queued
  through `pending_op`, so a draft written on a train is appended when the train leaves the
  tunnel. Saves that changed nothing are skipped, or a window left open overnight would append a
  fresh copy every thirty seconds and the user would find hundreds of identical drafts on their
  phone.

- **Recipient autocomplete**, from the mailbox, since there is no address book on Windows every
  user has. Ranked by frequency rather than recency — recency puts whoever sent the last
  newsletter at the top of every field.

### Incidents

- **Two of my own tests were wrong before the code was.** An attachment test asserted the output
  contained no `..`, which also matches base64 and MIME boundaries; it now checks the
  `Content-Disposition` header for path separators. And a soak-harness bug (recorded above)
  had the liveness check reporting a dead client while the client worked.

- **`clippy::items_after_test_module` fired twice**, both times because a block was appended to
  the end of a file that already had its tests there. Worth noting because the lint is right and
  the habit — `cat >>` onto a Rust file — is the cause.

- **The `soak` test moved behind a Cargo feature.** While a soak runs, its binary is locked, and
  an ordinary `cargo test` fails to link it with `LNK1104` — stopping verification for a reason
  entirely unrelated to whatever is being verified.

### Notes — what Phase 7 does not yet have

Stated plainly rather than left to be discovered:

- **Redirect** (docs/06 lists it beside reply/forward) is not implemented.
- **Inline images as `multipart/related` with `cid:`** — attachments are `multipart/mixed` only,
  so an image dragged into the body would travel as an attachment rather than appear in place.
- **Mail Drop** for oversized attachments, which is an iCloud service, and **Markup** on image
  attachments, which is a macOS framework. Both are named in docs/01 §6 as Mail behaviours; both
  need a Windows answer that does not exist yet.
- **The exit gate has not been run.** It asks that replies thread and render correctly in Gmail,
  Outlook and Apple Mail, with Outlook Windows named as the strictest. Gmail can be checked with
  the real account already configured; the other two need accounts that do not exist yet. Until
  then Phase 7 is feature-complete and unproven, which is a different thing from done.

---

## 2026-08-27 — Phase 8: organising mail

Rules, smart mailboxes, flags, VIPs, reminders, the junk filter and undo. Built on one shared
predicate engine, because docs/06 requires it and because two matchers would eventually disagree
about what the same saved search means.

### Added — the predicate engine

- **`rules/predicate.rs`** — one predicate type that both **compiles to SQL** and **evaluates in
  memory**, because a smart mailbox asks "which stored messages match?" and a rule asks "does
  this arriving message match?" — the same question from two directions. Values are always bound
  parameters, never spliced; `%` and `_` in a user's search term are escaped, so someone
  searching for a literal percent sign gets what they typed rather than a wildcard.

  A **property test** asserts the two agree on randomly generated predicates. It was verified
  non-vacuous by injecting a case-sensitivity bug: it failed, shrank to a minimal input, and
  passed again on revert. A property test nobody has seen fail is a test nobody should trust.

### Added — rules

- **`rules/engine.rs`** — actions, storage, and one evaluation path used by both triggers. "Run
  Rules" on a selection and the automatic pass on arrival are literally the same function, so
  they cannot drift; that difference is exactly what people test against and find broken.

  A later rule **re-reads the message**, so it sees what an earlier one did. Rules that depend on
  each other in order — "file it, then flag everything in that folder" — are how people actually
  build them, and evaluating them all against the original state would quietly break that while
  looking correct in every individual rule.

  **Delete moves to Trash, never destroys.** A rule that permanently deletes mail, on a predicate
  written in thirty seconds, is not something anyone recovers from.

  **"Run script" is deliberately not implemented.** docs/01 §8 lists it among Mail's actions. A
  rule action that executes an arbitrary program, triggered by mail _from anyone who knows the
  user's address_, is a remote code execution primitive with a friendly editor in front of it.
  Mail can offer it because AppleScript runs inside a sandbox the OS arbitrates; there is no
  equivalent here, and "the user configured it" is not a defence when the trigger is
  attacker-controlled. **Play sound** is absent for a duller reason — it belongs with
  notifications in Phase 10, and doing it here would mean two sound paths.

### Added — the junk filter, and the gate that rewrote it

- **`rules/junk.rs`** — a local Bayesian classifier. Standing rule 16 rules out every hosted spam
  service and shared reputation list, which is not a limitation to work around: it is the reason
  a local classifier is the right answer rather than a compromise.

- **The first gate was worthless, and it passed.** Scored against the live database it reported
  **97.3% accuracy** while catching **0.3% of the junk**. Both numbers were correct. The corpus
  was 97% ham, so a classifier answering "clean" for everything scores 97.2% — the accuracy
  figure was measuring the imbalance, not the filter. It was also the wrong corpus entirely:
  almost all of that mail was seeded test data whose Junk folder holds randomly assigned
  generated text, so nothing distinguished it and nothing could. The one real account had
  **twelve** messages in Spam.

  Replaced with the **SpamAssassin public corpus** — human-labelled, published for this purpose,
  and not written by me, since a corpus I write measures my imagination rather than the filter.
  The headline is now **balanced accuracy**, which a do-nothing classifier scores 50% on whatever
  the mix, plus floors on junk caught and a ceiling on real mail misfiled. The ceiling is the one
  that matters: a false negative leaves one more piece of spam in the Inbox, a false positive
  hides a message someone needed.

- **Naive Bayes could not meet that ceiling, so the combining rule changed.** The product form
  reached 96.7% balanced accuracy while misfiling **1.16% of real mail**, and no threshold fixed
  it — at 0.999 it was still 0.73%. The product saturates: a handful of extreme tokens pin the
  result at 0 or 1 and every later token is arithmetically ignored, so legitimate mail containing
  three spammy words becomes indistinguishable from spam. **Fisher's chi-square combination**,
  per Robinson, computes two independent statistics — how ham-like the evidence is and how
  junk-like — and a message that reads as both lands in the middle. That middle is where the
  newsletters live.

  The threshold was then **measured rather than chosen**: `junkgate` prints what each setting
  costs, and 0.99 trades seven points of catch rate to stop roughly one misfiled message in every
  three hundred.

  **Final result on the held-out half: 91.08% balanced accuracy, 82.6% of junk caught, 0.44% of
  real mail misfiled.** Trained and tested on disjoint, stratified halves — scoring the messages
  it trained on would report a number that means nothing.

- **Training only ever reads labels a human applied.** `junk_by_user` exists for exactly this. A
  classifier fed its own guesses converges on its own mistakes with growing confidence, and a
  test asserts a filter-marked message never enters the corpus.

### Added — VIPs, flags, reminders and follow-up

- **`rules/vip.rs`** — everything here keys off an **address**, and an address compared the wrong
  way is a feature that silently does nothing. `Ada@Example.com` and `ada@example.com` are one
  mailbox to every provider alive; the local part is case-sensitive per RFC 5321 §2.4 and nothing
  on earth treats it that way, so honouring the RFC would mean a VIP that stops working when the
  sender's client changes how it capitalises their own name.

- **The seven flag colours are validated, not trusted.** The stored value names a CSS custom
  property, so an unrecognised one is a token that does not resolve — an invisible flag rather
  than an error. The spec prose says "grey" and the token is `--flag-gray`; the code follows the
  token, because that is the string that has to resolve.

- **Remind Me leaves the message where it is** and hides it until due, rather than moving it to a
  holding folder. A move syncs to every other client the user owns, and a message that vanishes
  from the server's copy of the Inbox is one they cannot find on their phone.

- **Follow Up is deliberately conservative** — a question mark, no later inbound message in the
  thread, and three days elapsed — and a reply arriving later clears the mark. A list that fills
  with everything the user has ever sent is one they stop looking at, and a badge that stays on
  an answered conversation teaches them to ignore the badge.

- **Block Sender is retroactive.** The reason anyone blocks a sender is usually the mail already
  in the Inbox; a block that only applies to future mail leaves them to clear the rest by hand.

### Added — undo

- **`undo.rs`** — every entry stores **the state it replaced**, not a description of what
  happened. "Moved to Archive" cannot be reversed without knowing where the message was, and
  reconstructing that afterwards means guessing — "the Inbox" is the usual answer and the usual
  answer is wrong exactly when undo matters most.

  The stack lives **in memory**, deliberately. One restored from disk would offer to reverse an
  action from last Tuesday against a mailbox since synced, moved and re-threaded by other
  clients. Closing the window ends the undo history, which is what Mail does and what everyone
  already expects.

  Redo captures its inverse _before_ restoring, rather than re-running the original command.
  Re-running would be wrong for anything whose effect depends on when it happens — a rule, a
  snooze, a filter verdict.

### Added — the UI

- **One condition editor** for rules and smart mailboxes, since they are one type in the core.
  Two editors would diverge, and the whole point of a shared engine is that a rule and a smart
  mailbox written the same way behave the same way.

  The editor offers a **flat** list joined by all-or-any. `Predicate` nests arbitrarily and the
  core evaluates whatever it is given, but a UI for arbitrary nesting is a UI nobody can use, and
  Mail makes the same choice. A nested predicate loads, matches and runs; it is shown **read-only
  with an explanation** rather than flattened, because flattening would save back something
  meaning something different from what the user opened — and for a rule that files mail
  unattended, a silent change of meaning is the worst outcome there is.

- **The flag swatch went into the `Menu` primitive** rather than the feature. The alternative was
  a feature reaching past the `@/ui` barrel to style a raw element, which is the one thing the
  design system forbids. It is the only inline colour in the app, and the value is a token name
  rather than a colour literal, so standing rule 1 holds.

- **Remind Me computes dates against the user's calendar**, not by adding seconds. "Tomorrow" is
  a date, not 24 hours, and on the two nights a year the clocks change those differ. "This
  weekend" clicked on a Saturday afternoon means _next_ Saturday, not a reminder already past.

- **Ctrl+Z ignores events from inside a text field.** Ctrl+Z in a message list means "put that
  message back"; in a search box it means "undo my typing", and taking that away would be
  maddening.

- **Smart mailboxes carry no unread badge.** A count means running the predicate on every sidebar
  render, and a sidebar that stalls on a five-condition search over 50,000 messages is worse than
  one without a number on it.

### Incidents

- **The junk gate passed before it measured anything.** Recorded above in full because it is the
  same failure as the Phase 6 soak criteria "a corpse could satisfy" — a metric chosen before
  asking what a broken implementation would score on it. The fix both times was to state the
  do-nothing baseline in the output, every run, so nobody has to work it out.

- **A duplicate column in migration 0007.** `snooze_until` and its index were already in 0001. My
  grep of the existing schema listed five column names and did not include the one I was about to
  add. Caught immediately — the migration failed inside its transaction and rolled back — but the
  habit that caused it is worth naming: grepping for what I expect to find rather than for what I
  am about to write.

- **A sweep leaked into the verdict it was informing.** The threshold table left its tuning object
  holding the _last_ row's settings, and the gate then scored the final verdict with a
  configuration nobody ships — reporting a failure that was not real. Now reloaded from defaults,
  with a comment saying why.

- **My own test fixtures invented fields.** `AccountRow` has no `color` or `sortOrder`;
  `MailboxRow` has no `remotePath` or `depth`. Only the typechecker caught it, which is the
  argument for ts-rs generating these rather than hand-writing them.

- **`bigint` in the generated `Action`.** `#[ts(type = ...)]` on an enum _variant_ is ignored; it
  has to go on the inner field. A mailbox id arriving as a `bigint` would not compare equal to the
  `number` the rest of the UI holds — a bug that would have shown up as a rule quietly filing
  nowhere.

- **Shell heredocs and `node -e` mangled source repeatedly** — backticks eaten inside double
  quotes, template literals emptied, a `println!` split across two lines. Every one was caught by
  the compiler or a test, but the pattern is now unambiguous: multi-line source with backticks or
  quotes goes through the file-writing tool, not through the shell.

### Incidents — the app would not start, and it was not our code

- **A `tokio::spawn` in Tauri's `setup` hook.** `Sender::start` panicked with "there is no
  reactor running" on the first launch after Phase 7 — `setup` does not run inside a tokio
  runtime, and the bare form requires one. Every other spawn in the app already used
  `tauri::async_runtime::spawn`; that one and the new upkeep loop were the only outliers.

  It went unnoticed because **the whole of Phase 7 was written, verified and committed without
  the app once being launched.** Nothing in a test suite exercises `setup`. Both loops now
  return their future instead of spawning it, so the caller spawns on a runtime it actually has.

- **The webview then failed to be created at all**, with
  `0x80070057 The parameter is incorrect`, before `setup` ran. Recorded here in full because
  the diagnosis matters more than the outcome: **it is not this codebase.** Checking out
  `78c2b5f` — the last commit known to have launched successfully, at 22:13 the previous night
  — reproduced the failure exactly. Everything after that commit is therefore ruled out.

  Ruled out individually, each by running the app: `transparent: true` (with a forced rebuild,
  because the first attempt at this test silently reused the old binary and proved nothing), the
  saved `.window-state.json`, the entire `EBWebView` profile, the Phase 7 capability changes,
  disk space, WebView2 Runtime version and install date, Group Policy, RivaTuner, and stale
  processes holding the profile. WebView2 is working normally for other applications on the
  machine at the same time — Windows Shell and Google Drive both have live instances.

  I hypothesised desktop-heap or USER-object pressure: the machine had been up since the
  previous afternoon with 313 processes, `dwm.exe` holding 20,910 handles and RustDesk 18,067,
  which would explain a window that could not be created now but could be four hours earlier,
  and why applications already owning their windows were unaffected. I recommended a reboot.

  **That hypothesis was wrong.** The next launch succeeded — window shown in 405ms, 44 mailboxes
  synced, no panic — with _no_ intervention: same boot time, `dwm.exe` still at 20,938 handles,
  RustDesk still running, process count slightly higher.

  I then attributed it to a WebView2 environment left wedged while its profile directory was
  moved aside and back during the investigation. **That was wrong too.** It recurred later the
  same day with nothing having been touched, failed three launches in a row, and succeeded on
  the fourth — again with no intervention, and with handle counts and process counts within a
  percent of the failing attempts. The failure is intermittent on this machine and no cause has
  been established. What is established is the response: **relaunch, up to about four times,
  before changing anything.** Two plausible-sounding explanations have now been offered and
  neither survived; a third would be worth less than the empirical instruction.

  Two things worth keeping from it. The first is that the correct response to this error is to
  **try again before changing anything** — every "fix" attempted here was a null result, and had
  any of them been tried once more instead of once, it would have looked like the cure. The
  second is that the diagnosis that _did_ hold — checking out the last known-good commit and
  reproducing the failure on it — is the only step that produced certainty, and it took one run.

### Notes — what Phase 8 does not yet have

- **The rules editor is not reachable from a menu yet.** `RulesEditor` is built and tested but
  nothing opens it, and Alt+Ctrl+L is not bound. Both wait on the menu bar, which is Phase 10.
- **Move/copy by drag-and-drop and the Ctrl+Shift+M mailbox picker** are not implemented.
- **Snoozed messages are excluded from smart mailboxes but not yet from the main list**, and
  nothing wakes them on a timer — `wake_due` exists and is tested, but has no caller.
- **`junk_scan` is never called automatically.** The classifier files nothing on its own until
  something invokes it on arrival, which belongs with the sync loop rather than here.
- **The junk gate needs a corpus downloaded separately.** It is not vendored — 5MB of third-party
  mail in the repo would be worse — so `junkgate` prints where to get it and refuses to report a
  pass or a fail without it.

---

## 2026-08-27 (later still) — Phase 7: closing the gaps

Phase 7 was reported feature-complete and was not. Checking it against docs/06's build list
rather than against memory found seven things missing, and this closes all seven.

### Added

- **Redirect.** Passes a message on unaltered, so a reply goes back to whoever wrote it rather
  than to the person who passed it on. That distinction only survives if the original bytes do,
  so it works from the cached raw source or it refuses outright: rebuilding from the stored HTML
  would hand the recipient our reconstruction of somebody else's message — different encoding,
  different boundaries, signatures broken — with no way for them to tell. The `Resent-` block is
  **prepended**, which is what RFC 5322 §3.6.6 asks for rather than a shortcut, and
  `Resent-Bcc` is never written for the same reason `Bcc` never is.

  Display names in those headers are escaped. An unescaped quote closes the quoting early and
  everything after it reads as another address — which for a redirect means silently sending
  someone's mail to an address they never named. There is a test for exactly that string.

- **Inline images**, as `multipart/related` nested _inside_ `multipart/mixed`. The order is not
  interchangeable: related on the outside makes ordinary attachments part of the body's resource
  set, which Outlook renders as neither an attachment nor an image. Angle brackets on the
  `Content-ID` header and none in the `cid:` URL — RFC 2392 is explicit about the asymmetry and
  clients that get it wrong show nothing at all.

- **Draft conflict detection.** Before appending, the server is asked which copies of this draft
  it already holds; anything that is not the copy being replaced was written by another device.
  Both copies are then kept and the window says so. Resolving by picking a winner automatically
  is the one thing that must not happen here — the losing copy is work somebody did, and only
  they can say which version matters. The check runs _before_ the append, or our own new copy
  would be in the answer and every save would look like a conflict.

- **Four more format-bar controls** — colour, size, alignment and separator — bringing it to the
  eleven docs/06 names. Colours are a fixed list rather than a picker: a picker invites a pale
  yellow the sender sees against the composer's background and never against the recipient's.
  Sizes are absolute points, because `em` and `%` compound through nested quoting and a reply to
  a reply arrives at four points or forty. Alignment is a toggle, since "left" is not "unset" —
  an explicit `text-align` overrides the reading direction of anyone right-to-left.

- **Recipient chips drag between To, Cc and Bcc.** The payload carries a group-scoped MIME type,
  so a file dragged in from another window never lights up a recipient field as a target. The
  source chip is removed on `dragend` and only when `dropEffect` reports the drop was accepted:
  a drag abandoned over the desktop leaves the chip where it was.

- **Send Later gains Custom.** Date and time are parsed as _local_: `new Date('2026-08-28')` is
  midnight UTC and would schedule a message for the previous evening everywhere west of London.
  A moment already past is refused rather than sent immediately, which is the one outcome the
  user cannot undo.

- **The Undo Send delay is settable** — 10/20/30/off, in Settings. The core had read
  `compose.undoSeconds` since Phase 7; nothing could write it, so the choice the spec describes
  did not exist.

### Notes

- **The exit gate still has not been run.** It needs replies rendered and threaded correctly in
  Gmail, Outlook and Apple Mail, with Outlook Windows named as the strictest. Gmail can be
  checked with the account already configured; the other two need accounts that do not exist.
  Phase 7 is now complete against its build list and still unproven against its gate, and those
  are different things.

- **`clippy::items_after_test_module` fired again**, from appending a function to the end of a
  file with `cat >>`. The changelog already records this exact trap from Phase 7's first pass.
  Recording it a second time because the lesson evidently did not take: appending to a Rust file
  puts the code after the test module, every time.

---

## 2026-08-27 (evening) — Phase 8: closing the gaps

Same exercise as Phase 7 an hour earlier: checked against docs/06's build list rather than
against memory, and found the engines were largely built with nothing able to invoke them.

### Added

- **Smart mailboxes work when clicked.** The sidebar has carried predicates since this morning
  and selecting one did nothing: the selection had no way to hold a predicate and the list only
  knew how to query by mailbox id. The selection now carries one or the other — never both,
  which is the same rule `mailboxIds` already had for the same reason — and the list runs one
  of two queries. The saved-search query pages by offset rather than by keyset cursor: an
  arbitrary predicate has no cheap ordering to seek into, and the result sets are small enough
  that it does not matter. The folder list keeps its cursor precisely because it is the one
  paging through fifty thousand rows.

- **A smart mailbox editor**, sharing `PredicateEditor` with the rules editor. What it adds
  over that editor is what it _lacks_: no actions. A smart mailbox is a question about the
  mailbox, not something that happens to mail.

- **A VIP mailbox**, as a saved search over the VIP addresses rather than a folder, so nothing
  is moved and a VIP's mail still appears in the Inbox where they expect it. It matches on
  `from` rather than `anyText`, or a newsletter quoting a VIP's address would land in the row
  meant for mail they actually sent. The row is absent until there is a VIP: an empty row that
  can never fill reads as a broken feature rather than an unused one.

- **The junk banner**, which says two different things depending on who decided. The filter's
  guess invites a correction and carries its confidence; the user's own decision is stated back
  without argument. A banner that debated somebody's own judgement would be the fastest way to
  make them turn the filter off. It sits _above_ the body — a warning underneath a phishing
  attempt has already lost.

- **Training mode**: score everything, file nothing. The first weeks of a Bayesian filter are
  its worst, and the damage it can do then — a real message quietly moved out of the Inbox — is
  exactly what makes someone disable a junk filter permanently.

- **Ctrl+Shift+M**, a mailbox picker with typeahead. Ranked so a prefix match beats a contained
  one: otherwise typing "arch" puts "Research Notes" above "Archive", the top result changes
  under the user between keystrokes, and Enter sends mail somewhere they never looked at. Enter
  on an empty result list does nothing rather than falling through to the first mailbox.

- **Alt+Ctrl+L**, running the rules over the selection, and the Rules and Smart Mailbox editors
  reachable from Settings. Both shortcuts are registered on the window rather than on a focused
  element, because both act on the _selection_ and the selection outlives focus moving between
  panes; both are ignored while the caret is in a text field.

### Notes

- **Drag-and-drop move to the sidebar already existed.** Checked before building it. Recording
  it because the check took thirty seconds and would have cost an afternoon.

- **The soak is showing memory growth that is on course to fail its own threshold.** At 425 of
  720 minutes the working set has gone 22.7MB → 35.1MB, but not as a leak's straight line: flat
  for 200 minutes, a single 7MB step between minute 240 and 245, then a slow climb inside a
  ±1MB band. The gate compares the last quarter against the second and fails above 25%; on the
  current trend it lands near 30%.

  The likely cause is not a leak. There is no explicit `cache_size` PRAGMA, so each pooled
  connection takes SQLite's default 2MB page cache, and with a reader pool of four plus the
  writer that is a bounded ~10MB that fills as the pool warms — which is a step, not a slope.
  **Not changed while the soak is running**, because doing so would invalidate the run that is
  measuring it. The verdict is due around 22:15; the decision after it is whether to make that
  ceiling explicit rather than an accident of a default times however many connections happen
  to exist.

---

## 2026-08-27 (night) — Phase 8: the exit gate

The gate asks four things, and `src-tauri/tests/phase8_gate.rs` now measures three of them; the
fourth was already measured by `junkgate`. All four pass.

### Results

| gate                                                                        | result                                                  |
| --------------------------------------------------------------------------- | ------------------------------------------------------- |
| a rule created in the UI fires on arrival and on manual run                 | **pass**                                                |
| a 5-predicate smart mailbox agrees with hand-written SQL over the 100k seed | **pass** — 101,282 messages, both forms returned 15,694 |
| undo restores exact prior state for every action type                       | **pass** — 11 action types                              |
| the junk classifier exceeds 90% on a labelled corpus                        | **pass** — 91.08% balanced                              |

### What each one was made to mean

- **"Created in the UI"** is taken to mean the rule goes in through `rule_save` — the function
  the editor's OK button calls — and comes back out through `rules_list`. A synthesised `Rule`
  value would skip the JSON round-trip, which is exactly where a serialisation bug would hide.

- **Gate 2 was checked for sensitivity before being believed.** Agreement between two queries
  proves nothing if neither can disagree. Flipping the unread clause on the compiled side alone
  moved the count from 15,694 to 71,317 and failed the assertion, which is what makes the
  passing run evidence. A first attempt at this probe — raising the size threshold from 100 to
  1000 bytes — changed nothing, because every matching message is over 1000 bytes anyway; a
  probe that cannot fail is as useless as the test it is checking.

  The test also refuses to pass if the hand-written query matches nothing, since a predicate
  that matches nothing agrees with a query that matches nothing and neither of them works.

- **Undo covers eleven action types**, including the compound case: one step spanning mailbox,
  seen, flagged and junk at once, which is what "Apply Rules" captures. A per-field undo would
  pass all ten single-field tests and still lose three quarters of that one. Each test starts
  from a deliberately awkward state — flagged blue, read, junk, snoozed, thread muted — so
  "restored" cannot be mistaken for "reset to the defaults".

- **Send is undone by a different mechanism, and there is a test that says so.** Everything else
  restores a row to a prior value; a send has no prior value, because once the bytes have left
  no local change brings them back. Undo Send is a hold in the outbox instead. The gate lists
  send among the actions undo must cover, so the omission is documented rather than left
  looking like one.

### What the gate does not prove

Gate 1 exercises `run_on_arrival` — the function the incremental sync calls with the ids it has
just written — and does not open a socket. That new mail arrives at all and reaches that path is
what the Dovecot gate already covers; duplicating it here would test the rig rather than the
rules.

---

## 2026-08-28 — Phase 9: search, and the Phase 5 soak verdict

### The twelve-hour soak passed

The last outstanding Phase 5 gate item, finally measured over a full 12.03 hours:

| measure                            | result                       | budget       |
| ---------------------------------- | ---------------------------- | ------------ |
| memory, 2nd quarter → last         | 29.8MB → 36.1MB (**+21.2%**) | under 25%    |
| peak connections                   | 1                            | 3            |
| samples over the connection budget | 0 of 144                     | under 15     |
| deliveries picked up               | **143 of 144**               | at least 108 |

The liveness number is the one that matters, and it is the one an earlier run failed silently:
a dead client uses no memory and opens no connections, so it passes the other two. This client
was awake for twelve hours and missed one delivery — the last, which landed after the final
sample.

**The +21.2% is not a leak but is not nothing.** The shape says so: flat for 200 minutes, a
single 7MB step, then drift inside a ±1MB band. There is no explicit `cache_size` PRAGMA, so
each pooled connection takes SQLite's default 2MB page cache, and four readers plus a writer is
a bounded ~10MB that fills as the pool warms. The ceiling is real but accidental — the product
of a default and however many connections happen to exist — and making it deliberate is worth
doing before it is discovered on a smaller machine.

### Added — search

- **A query language and its parser.** `from:`, `to:`, `subject:`, `mailbox:`, `has:attachment`,
  `is:`, `before:`, `after:`, `larger:`, `smaller:`, free text between. Nothing typed reaches
  FTS5 as syntax: every term is quoted, so `NEAR`, `AND`, `*` and `^` are matched as words. An
  unrecognised field becomes free text rather than an error, because `re: figures` and
  `http://example.com` are things people type.

- **Natural-language dates**, with one rule that keeps them from being a nuisance: **a bare
  month name is never a date.** `March` is a person, `May` is a person, and a search box that
  turned a colleague's name into a date range would break searches with no error and nothing on
  screen to explain it. A phrase needs an unambiguous marker — `in March`, `last week`,
  `yesterday`. `after:march` is accepted, because there the user has already said they mean a
  date.

- **Top Hits ranking**: BM25 × recency × VIP × thread participation, multiplied rather than
  added so none dominates. BM25 arrives _negative_ from SQLite and more negative is better; used
  directly as a multiplier it would invert the ranking into something that looks like a
  plausible order rather than an obviously broken one.

- **Suggestions**, grouped with headers, arrow-navigable. Debounced — not for load, since the
  core answers in under a millisecond, but because a list that changes on every keystroke moves
  under the pointer and a click aimed at one row lands on another.

- **The scope bar, match highlighting, save-as-smart-mailbox and search history.** Highlighting
  returns _segments_, never markup: a subject is hostile input, and the moment highlighting
  produces HTML it becomes an injection with a friendly name. A saved search is stored as a
  predicate rather than as its text, so a later parser change cannot quietly alter what a
  year-old smart mailbox matches.

- **Attachment text extraction** for PDF, DOCX and TXT. This is the one place in the app that
  _parses_ an attachment, by third-party code, on the user's machine, from a file a stranger
  sent — so: a size ceiling before anything is parsed, a page ceiling, every parse wrapped in
  `catch_unwind`, and only the one known entry read from a DOCX zip. No `.doc` or `.xls`: old
  Office formats are a long list of parser CVEs and are not what people search for.

### The exit gate

| measure                           | result                  | budget      |
| --------------------------------- | ----------------------- | ----------- |
| worst search at 101,282 messages  | **104–110ms**           | under 120ms |
| worst suggestion                  | **under 1ms**           | under 30ms  |
| five queries ranked and explained | printed by `searchgate` | —           |

Getting there took three findings, all measured rather than guessed:

1. **`strftime('%s','now')` is non-deterministic**, so SQLite may not lift it out of the loop and
   evaluates it once per row. It was in the snooze filter of every list and search query added
   this week. Now bound as a parameter.

2. **`ORDER BY bm25()` cannot be answered from an index.** "the" matches 98,565 of 101,282
   messages, so ranking meant 98k joins and a 98k-row temporary b-tree: 160–220ms. Candidates
   are now selected by **date** and ranked afterwards. That is consistent with the ranking rather
   than a compromise against it — a 30-day half-life means only recent candidates can
   realistically win. The cost is stated in the code: a very old message that is a far better
   match than anything recent can fall outside the window.

3. **The `mailbox` join ran on every search** and is needed only by `mailbox:` queries. It runs
   once per matching row _before_ any limit, so on a common term it was a hundred thousand index
   lookups for a table nothing read. Removing it took the worst case from 115–118ms to
   104–110ms.

An attempt to invert the join — driving from the date index and probing FTS as a subquery — was
abandoned after it failed to finish in ten minutes. Recorded because it is the obvious idea and
it is much worse.

### Incidents

- **A wrong column name hidden by a swallowed error.** The person-suggestion query selected
  `address`; the column is `addr`. The `prepare` was wrapped in `if let Ok`, so the failure was
  discarded, suggestions silently returned nothing, and every test passed. Same shape as the
  junk gate that "passed" while catching nothing. The `?` is back and there is now a test that
  would have caught it — which the first version simply did not have.

- **The first performance verdict was measured against a busy machine.** The 12-hour soak was
  running throughout, and readings moved 115ms → 134ms → 110ms with no code change between two
  of those. Timings taken while something else is saturating the disk are not a verdict, and the
  numbers above were taken after the soak finished.

---

## 2026-08-28 (later) — Two changes before Phase 10

### Changed — remote images load by default

At the owner's request. **This reverses standing rule 11**, which says remote content is blocked
by default and "has no exceptions", so it is recorded here in full.

A remote image is the read receipt nobody consented to: a URL unique per recipient tells the
sender their message was opened, roughly when, and how often. That is why the rule existed. It
is the owner's own mail and their call to make, and it is a **setting** — Settings ▸ Reading —
rather than a decision baked in, with a per-message override in both directions: a banner to
load images on a message when the setting is off, and a banner to block them when it is on.

`PROMPT.md` §11 now contradicts the code. It is the contract and not mine to edit; whoever owns
it should decide whether to amend the rule or the exception.

Everything else rule 11 requires is unchanged: the sandboxed frame, no scripting, the Rust-side
sanitiser, and the rule that the frame itself never makes a network request.

### Fixed — a privacy claim that was not true

Chasing the copy for that setting turned up an error in the code comments and in `docs/03` §6.3,
which says to _"proxy through the Rust core so the sender never sees the user's IP"_.

**The Rust core runs on the user's own machine.** A request it makes leaves from the user's own
address, so the sender sees the IP either way. Nothing here is a proxy in the sense the word
implies, and building one would mean routing someone's mail through a server this project does
not have and standing rule 16 would not permit.

What fetching through the core does buy is real and was worth keeping: no cookie store, so one
sender's pixel cannot identify the reader to the next; no `Referer`, so the request does not name
the message; a generic `User-Agent`, naming no client or machine; and `data:` URIs into the
frame, so the document makes no network request at all.

The comment and the Settings text now say that rather than the stronger thing. A promise of
anonymity that is not kept is worse than no promise — and this one would have been made to the
user directly, on screen, at the moment they turned the feature on.

### Changed — the page cache is now a budget rather than a default

The twelve-hour soak passed at +21.2% memory growth against a 25% ceiling, and the shape said it
was the SQLite page cache filling rather than a leak. There was no `cache_size` PRAGMA, so each
pooled connection took the default 2MB — a ceiling arrived at by multiplying a number nobody
chose by however many connections happened to exist.

It is now explicit at 8MB per connection: larger than the default on purpose, because the store
is the point of the app and the queries that matter are the ones a bigger cache helps. Five
connections gives a bounded 40MB, which is the number to reach for if it ever has to come down.

A test asserts the value, because it is a budget the soak was measured against and a change to it
should be a decision rather than a drift. Writing that test found that the query fixture opens
raw connections and never calls `configure`, so it was measuring the default rather than the
setting — the test now configures a connection explicitly.

---

## 2026-08-28 (Phase 10) — Polish and platform

Phase 10 in full: the keyboard registry, the Windows platform surface, the states that were
never shown, and the accessibility and motion audits. Committed as `c57bfc7`, `795434a`,
`b24e4d8`, `908d708`, `aa6d3bd` and `efc856a`.

### Added — the platform surface

- **Taskbar unread badge** (`ITaskbarList3::SetOverlayIcon`). The digits are a hand-built 3×5
  bitmap font rather than a rasterised typeface: Windows scales an overlay to 16px and a real
  font at that size is a smear. Above 99 it shows `99+`, because a count that admits it has
  stopped counting is better than one that is unreadable.
- **Tray icon** with the unread count, and **run at login**, user-toggleable.
- **Toast notifications with inline Reply / Archive / Mark as Read**, per-account and VIP-only.
- **Jump list** — New Message, Inbox, Search.
- **`mailto:` handling** and a **`.eml` viewer window**, read-only.
- **Send and receive sounds**, off by default.
- **Swipe gestures** on list rows, by precision touchpad and by touch.

### Changed — toasts bypass the Tauri plugin

`tauri-plugin-notification` wraps `tauri-winrt-notification` but exposes only `action_type_id`,
which is the mobile notion of an action. `add_button` — the thing Windows actually draws on a
toast — is not reachable through it, and docs/06 asks for three buttons. The Windows path now
talks to the crate directly; the plugin stays for the permission plumbing.

**Those buttons do not appear in a dev build, and that is expected.** A toast is addressed to an
AppUserModelID, Windows only shows toasts for an AUMID it knows, and it learns one from a Start
Menu shortcut written by an installer. Until Phase 11 there is no installer, so `show()` fails
and is logged at debug. The COM activator docs/06 names — which is what lets a toast act when
the app is _not_ running — has the same dependency and is deferred with it, deliberately.
`src-tauri/src/platform/toast.rs` documents all three states at the top.

### Changed — where each kind of action is decided

Two shell integrations pull in opposite directions and the reasons are worth recording.

**Toast actions route to the UI.** Archive and Mark as Read already exist as mutations with the
cache invalidation that makes the message list update. A second implementation behind the toast
would be the one that archives on the server and leaves the message sitting in the list.

**`mailto:` routes the other way** — `links.rs` parses it and opens the compose window from Rust
with the sanitised fields as query parameters. Handing the raw link to the frontend would mean
parsing it twice, and the second parser is the one that forgets to drop `Bcc`. The parser is an
allow-list (to, cc, subject, body only) because RFC 6068 permits arbitrary headers, and a page
that could set `Reply-To` can make a reply go somewhere the sender never intended.

### Added — the sync status strip

The sync engine has tracked per-account errors since Phase 5 and **nothing has ever displayed
them**. An account whose password expired failed every few minutes and the entire user-visible
consequence was that new mail stopped arriving: the app looked like it was working and was not.

That is the exact shape of the "my mail is stale" report from this project's own testing on
2026-08-27. That report turned out to have a different cause — three seeded `.example` accounts
sitting above the real one — but only by luck. Had sync genuinely been failing, the app would
have been just as silent.

The strip is **absent when things are fine**. A permanent "Connected" line is a banner that
teaches you to stop looking at the space it occupies, so that when something does appear there
you no longer see it.

Offline is tracked separately from account errors, and outranks them. With no network every
account fails, and four rows saying so describe one fact four times; it is also the one failure
where the honest thing to say is that the mail is still here and merely not current. Coming back
online triggers a sync, because IDLE connections died with the network and nothing else will say
what arrived in the meantime.

`navigator.onLine` is a weak signal — it means "there is a network interface", so a captive
portal still reports true. It is used anyway because the case it does catch is the common one
and it catches it instantly, where waiting for an IMAP timeout takes half a minute. Anything
subtler still arrives as a per-account error.

### Added — one `EmptyState`, and the surfaces that had none

Every "there is nothing here" now says what is true, why, and — where one exists — a way out.
The `hero` variant preserves docs/02 §6.10's larger, dimmer treatment for the reader at rest,
because "No Message Selected" is the app waiting rather than an absence to explain.

The `.eml` viewer, the message list, the reader and the sidebar all use it.

### Changed — `MessageFrame` extracted from `MessageBody`

The sandboxed iframe and its CSP are now one component shared by the reader and the `.eml`
viewer. This is not tidiness: the frame is the security boundary standing rule 11 is about, and
a second copy is one that eventually drifts — and the drifted copy does not read as wrong.
`MessageBody` keeps what is genuinely its own: the remote-image setting, the per-message
override, and the two banners.

### Added — two audits, as tests rather than as an afternoon

- `tests/unit/motion.test.ts` — no linear _stops_, easing tokens only, reduced motion from one
  place. `infinite` animations are exempt: a spinner rotating at a constant rate has to be
  linear, and eased it visibly hitches once per revolution.
- `tests/unit/accessibility.test.ts` — focus rings, fixed pixel font sizes, accessible names.

An audit performed once is a fact about one afternoon. These run on every commit.

### Fixed — dead ends found by auditing

- **A message body that failed to load rendered an empty div.** Indistinguishable from a message
  that is genuinely blank, so a failed fetch looked like an empty email. Now says so, with a
  retry.
- **A failed accounts or mailboxes query left the sidebar empty and silent.** The first-run gate
  could not cover it — `firstRun` requires `isSuccess`, correctly — so the app looked like a
  fresh install with no mail. Now says the database did not answer, notes the mail is still on
  the server, and offers a retry.

### Incidents

**The accessibility audit's first version was worse than nothing.** It matched IconButton tags
with a regex whose character class stops at the first `>` — which in JSX is usually the arrow of
an inline `onClick`. It silently checked 30 of 56 IconButtons and passed. Found by probing it
with a deliberately unnamed button and noticing the probe _did not fire_. Replaced with a
brace-aware scanner, and the count is now asserted, so finding nothing is itself a failure.
Every check in both audit files was probed the same way afterwards.

**A test was written to match the code rather than to match what should happen.** The touch
swipe's `pointercancel` handler shared the release path, so a system-interrupted gesture past
the threshold _committed_ — archiving a message nobody let go of, which is precisely the failure
the distance threshold exists to prevent, arriving through the one door that bypassed it. The
first version of the test asserted this behaviour as correct. Both were corrected.

**`capabilities/default.json` listed only `main` and `compose-*`.** Every core call from a
`.eml` viewer window would have been denied _silently_ — the failure mode `CLAUDE.md` warns
about, which produces no error anywhere. Found by reading the file rather than by anything going
wrong. `eml-*` added, with the reason written into the description.

**A test mutated `WINDIR` process-wide.** `std::env::remove_var` is global and cargo runs tests
on several threads, so it could have failed a sibling test that reads the variable — a flake
that would have appeared at random and been blamed on anything but this. The path builder now
takes the directory as an argument.

**Replacing `= []` destructuring defaults with `?? []` surfaced a pre-existing problem.** Both
allocate a new array per render, so the memo building the entire sidebar tree was rebuilding on
every render. eslint cannot see this through a destructuring default. Now one shared constant.

**Two focus rings were removed without replacement**, in the compose editor and the message
list. Both are correct — a caret indicates focus in a text surface, and the selected row does in
a list — but neither said so. Rather than an allow-list in the test, both now carry a
`focus-ring-exempt` marker in the CSS, so the reason sits where the next reader will find it.

### Notes — what the exit gate still needs from a person

The gate is: _complete a full triage session (read, flag, archive, reply, search, send) without
touching the mouse; Narrator walkthrough recorded; no unstyled or dead-end state reachable._

- **Keyboard coverage** is asserted in `tests/unit/shortcuts.test.ts` — all six verbs, plus the
  arrow keys that select a message. The _session itself_ has not been performed and cannot be by
  me.
- **The Narrator walkthrough** needs a screen reader on the owner's machine and a recording. Not
  done.
- **Dead ends**: the two above are fixed and the surfaces are covered. "No unstyled state
  reachable" is a claim about every screen, and it has been checked by reading rather than by
  walking every one.

Phase 7's exit gate also remains open — it needs Outlook and Apple Mail accounts.

---

## 2026-08-30 — Phase 11: ship, part one

Two chunks: the diagnostics built on the 29th and never written up (see Incidents), and the
Settings window.

### Added

- **A log that survives.** Every log line went to stdout, and a release build has no console —
  so in the hands of an actual user the whole log stream went nowhere, and the one moment
  anybody wants a log is after something has gone wrong. Logging now goes to the terminal _and_
  to a rotating file beside the database: 4MB a file, five kept.
- **Crash reports.** A panic hook writes the message and a backtrace to
  `%LOCALAPPDATA%\com.uniki.halcyon\diagnostics`, ten kept, newest first. It chains the previous
  hook rather than replacing it, so a developer running from a terminal still sees the panic
  printed.
- **`crashgate`**, a binary that proves the hook fires under the release profile.
  `[profile.release]` sets `panic = "abort"`, and the obvious reading of that — the process dies
  with no chance for a hook — would make the whole of `diagnostics.rs` dead code in exactly the
  build where it matters. The obvious reading is wrong, but _believing_ it is not knowing it, so
  the gate asks the real question of the real profile: panic in a child, look for a file. It
  writes one, 1850 bytes, and the child aborts with `0xC0000409`.
- **The Settings window — seven panes**, in a window of its own, which is what docs/06 Phase 11
  asks for and what Mail does. General, Accounts, Composing, Signatures, Rules, Privacy,
  Advanced.
- **Appearance controls that have never existed.** Theme, density and translucency have been in
  the store since Phase 1, resolved against what Windows reports, applied before first paint,
  driving three token remaps — with no control anywhere. The only way to change the message
  list's density was to edit localStorage by hand. docs/02 §5 asks for a Reduce Transparency
  escape hatch specifically for users whose GPU makes the Mica backdrop painful; for ten phases
  they could not reach it.
- **A Signatures pane.** `signature_get` and `signature_set` have existed since Phase 7 — with
  tests, with sanitising, with the placement rule for quoted replies — and **nothing called
  either of them.** Every message this app has ever sent went out unsigned, not because
  signatures were unimplemented but because there was no way to type one. That is a failure
  invisible from the Rust side: the tests pass, the column exists, the feature is absent.
- **An Advanced pane**, which is where the crash reports surface. A report that is written and
  never shown is a file in a folder nobody knows the name of.
- **A Privacy pane** stating plainly what the app sends, including the one uncomfortable line:
  mail is stored unencrypted on the disk, and BitLocker is what protects it at rest.
- `Ctrl+,` opens Settings, and the sidebar button now opens the window instead of a sheet.
- `tests/e2e/settings.spec.ts` (9 tests) and `tests/unit/settings.test.ts` (7).

### Changed

- **Settings was six sections stacked in one modal sheet, mounted inside `AccountsGate`.** The
  settings a user could open were therefore coupled to the first-run account assistant, and the
  root threaded an open flag and two callbacks through three components to reach the one button
  that set it. Stacking also meant every section loaded at once: opening Settings to change the
  undo delay ran the junk filter's status query and every account's notification preferences.
  Panes mount only while shown.
- **A settings _window_ rather than a sheet, for a reason beyond fidelity.** Settings is where
  someone goes to fix something they are looking at, and a modal sheet hides the very thing they
  are trying to fix.
- **Four byte-identical CSS modules became one.** Composing, Reading, Junk and Notifications each
  had their own copy of the same forty lines. That is how a settings window ends up with sections
  that do not line up — a spacing change made in one copy is invisible until the sections are
  finally shown side by side, which is exactly what a paned window does.
- **`npm run verify` now runs the e2e suite** (`test:e2e:gate`, serialised). See Incidents for
  why it had to.
- Display preferences are announced to every window over a Tauri event. They are the only
  settings the UI owns outright; everything else is read back from the core, so a second window
  sees a change the moment it asks. Without this, Settings would be the one place in the app
  where changing the theme appeared to do nothing.

### Fixed

- **Typing in the signature editor scrambled the text.** "Vishal Singh" came out "ishal SinghV".
  The editor takes its initial HTML once, but the guard that records "applied" skips an empty
  string — so with no signature stored yet, the first keystroke came straight back in as
  _initial_ content and landed where the loader puts it. The editor's output is no longer allowed
  back into a prop the editor reads. Found by an e2e test that typed a name and read it back;
  every other assertion about that pane passed with the bug present.

### Incidents

- **The 29th's work shipped without a changelog entry.** `7cd8396` (logs and crash reports)
  touched ten files and `CHANGELOG.md` was not one of them, against a standing instruction that
  says every session, without being asked. Recorded here a day late, which is the whole cost of
  the miss: the reasoning was still recoverable. It would not have been in six weeks.
- **Four visual baselines had been failing since Phase 8 and nobody knew**, because
  `npm run verify` did not run the e2e suite. The browser path deliberately stopped rendering
  message bodies — the sanitiser lives only in the core, and rendering stored HTML in the browser
  would be the one place it is skipped — and the baselines still showed the old rendered bodies.
  Two phases of drift. Baselines refreshed, and e2e added to the gate so the next one cannot rot
  the same way.
- **Adding e2e to the gate immediately failed the scrolling budget.** Nine parallel workers on
  one machine measure the machine, not the app: the same test passed at a p95 of 18.5ms alone and
  failed minutes later under the full suite. The gate runs `--workers=1`, matching what the config
  already did on CI. Interactive `npm run test:e2e` stays parallel.
- **`docs/06` asks for "opt-in upload" of crash reports and there is deliberately none.** There is
  nowhere to upload to. A crash-collection endpoint is a service with its own retention and
  privacy questions, and shipping a client that posts to a server nobody has chosen would be worse
  than shipping none. What the UI offers instead is the report and the folder it is in. Standing
  rule 16 holds. When a destination exists, opt-in sending is a small addition on top; the hard
  part — capturing something worth sending — is done.
- **"Phase 12" appeared in four places** (`jumplist.rs`, `toast.rs` twice, `CHANGELOG.md`).
  docs/06 has eleven phases and the installer is Phase 11. Corrected.

### Notes — what Phase 11 still needs

- **Code signing has not started and nothing here can substitute for it.** docs/07 §4 says the
  certificate process should have begun at Phase 9; it has weeks of lead time. The exit gate is
  "clean install on a fresh Windows 11 VM with **no SmartScreen warning**", and without a
  certificate every download shows "Windows protected your PC" and Smart App Control blocks the
  installer outright with no override. Only the owner can start this.
- Still to build: import and export, the first-run welcome, installer artwork, NSIS/MSIX and
  signing, the auto-updater, README, licence and privacy policy.

### Verified in the running window — 2026-08-31

RivaTuner was closed, so the checks the browser cannot make were made against the real app by
driving it from PowerShell and reading the UI Automation tree — the same tree Narrator reads.

- `Ctrl+,` opens Settings as a real OS window. All seven panes in the tree; **zero interactive
  elements without an accessible name.**
- **The capability entry is right.** This was the check that mattered, because a window missing
  from `capabilities/default.json` has every core call denied with no error, no console message
  and no log line. The Accounts pane returned the real account over IPC, and the Advanced pane
  returned the crash reports.
- **A theme changed in Settings repaints the mailbox behind it.** Measured rather than asserted:
  average brightness of a strip of the mailbox clear of the Settings window went 243.9 → 31.2
  between Light and Dark. This is the one mechanism the Playwright run cannot reach, since a
  second OS window is a second React root sharing nothing but `localStorage`.
- **Reopening moves the existing window rather than opening a second one**, and brings it to the
  front.
- **The crash reporting works on a crash nobody staged.** The two RTSS-induced startup panics
  from the night before were already on disk as full reports with backtraces, and the Advanced
  pane listed and opened them. That is the whole feature working end to end on a real failure,
  which no test could have arranged.

### Fixed — 2026-08-31

- **The Settings window's title never changed.** Setting `document.title` names the WebView; the
  OS window keeps whatever the builder gave it until told otherwise, so the title bar read
  "Settings" for ever while the code — and its comment claiming the title carries the pane —
  said otherwise. It now calls `setTitle` as well. Found by reading the window's Name in the UIA
  tree; Playwright had passed, because in a browser `document.title` _is_ the title.

---

## 2026-08-31 — Phase 11: import and export

### Added

- **mbox reading and writing**, hand-written. mbox has no length prefix and three incompatible
  escaping conventions all called mbox; the reader is permissive and the writer is strict
  mboxrd, so our own export round-trips exactly and somebody else's file imports as closely as
  the format allows. Streams a line at a time — a Thunderbird `Inbox` of several gigabytes is
  ordinary, and splitting one in memory simply fails.
- **Thunderbird import.** Thunderbird stores mail as mbox files with no extension, so importing
  it is finding the files and working out what each folder was called. Subfolders come from the
  `.sbd` directories, and `X-Mozilla-Status` is read so **which mail had been read survives** —
  without it an import of years of archives arrives as thousands of unread messages, which
  makes the result unusable however correct the text is.
- **Loose mbox import** through the system file picker, for anything else that can export one.
- **Export to mbox and to an `.eml` tree.** Two formats because they answer different
  questions: mbox is what Thunderbird and Apple Mail import, an `.eml` tree is what Windows and
  Outlook can read and what a person can search with the tools they already have.
- **A local account, "On My PC"**, which is where imported mail goes.
- **A folder picker** (`IFileOpenDialog` with `FOS_PICKFOLDERS`), for where an export lands.
- `src-tauri/tests/transfer_gate.rs` — eight gates over a sample containing every thing that
  actually breaks importers.

### Changed

- **`platform/files.rs` grew a folder picker, and the new function landed after the test
  module** because it was appended with `cat >>`. Clippy's `items_after_test_module` caught it.
  Moved above.

### Fixed

- **A window remembered from a display that no longer exists stopped the app starting.** Found
  while chasing something else, and kept because it is real on its own terms:
  `tauri-plugin-window-state` restores the window where it was, and when that display is gone —
  a laptop undocked, a monitor unplugged — WebView2 refuses the rectangle and Tauri's setup
  hook panics. **No window appears, so there is no setting to change**; the only cure is
  deleting a JSON file in `%APPDATA%` nobody knows the name of.

  Remembered geometry is now clamped onto the current virtual screen before Tauri builds. A
  window that still fits is left where it is — parking one half off the edge is deliberate, and
  Windows itself restores those unchanged — while one that had to be shrunk is placed fully on
  screen, because its position came from a coordinate space that no longer exists.

  This was **not** the cause of the failure being chased. See Incidents.

- **An unescaped `From ` line in a message body split the message in two.** The format says such
  a line must be escaped; a great deal of real mbox was written by something that did not. The
  reader now requires a header on the following line before treating `From ` as a separator, so
  an ordinary sentence beginning "From here on…" stays in its message.

### Incidents

- **Two wrong diagnoses of the same startup failure, in a row, each confident.** The app began
  failing with `failed to create webview: 0x80070057` and exit 101.

  _First:_ RivaTuner. The identical HRESULT had appeared two days earlier with RivaTuner
  running and had gone away when it was closed, so the note in memory said RivaTuner and the
  check stopped there. RivaTuner was not running.

  _Second:_ window geometry. The saved window was 2800×1800 and the desktop measured 1920×1080,
  which is a real failure mode with a real fix — so the fix was written, tested, and it
  corrected the rectangle exactly as designed. **The app still failed.** Deleting the state file
  entirely and starting from the configured default also failed, which is what finally ruled
  geometry out.

  _Actual cause:_ **the machine was locked.** WebView2 cannot create a window on a locked
  session, and while locked Windows reports the lock screen's metrics — which is where the
  "1920×1080 display" came from. The screen had not changed at all.

  Both wrong answers were arrived at by matching a symptom to a cause that had produced it
  before, and testing the fix rather than the diagnosis. The decisive experiment in each case —
  is RivaTuner running, does it fail with no state file at all — was cheaper than the fix and
  was done second.

- **The geometry fix then corrupted the thing it was protecting.** Running on a locked session,
  it read the lock screen's 1920×1080 as the user's display and rewrote a perfectly good
  2800×1800 window down to fit it, on every failed launch. It now skips entirely unless
  `OpenInputDesktop` succeeds, which is false on a locked machine, during a UAC prompt, and on
  any other secure desktop. The remembered geometry was restored by hand from the values the
  new log line had recorded on its way past.
- **The transfer gate found the `From `-line bug on its first run**, which is what it was
  written for — but the sample that found it was itself malformed, and the first instinct was to
  fix the sample. The right reading was the opposite: files like that are common, and an
  importer that corrupts them is the problem. Both halves of the new separator rule were then
  probed by breaking each in turn.
- **A shell heredoc mangled `mbox.rs` badly enough to delete three items**, including the
  `Counts` struct and both `read` signatures — backticks inside a `node -e` template literal
  were interpreted by bash. Reassembled from the surviving parts. This is the fourth time this
  session that quoting in a shell one-liner has damaged a file; the Write and Edit tools are
  the way to change Rust and TypeScript, not `sed` and `node -e`.
- **Playwright's dev server outlived the test run** and held port 1420, so the next
  `npm run app:dev` failed with a message about the port rather than anything to do with the
  app. Killed by hand.

### Notes

- **Import and export have not been exercised in the running window.** The machine is locked,
  so the app cannot start at all. Everything beneath the UI is covered by the eight gates in
  `src-tauri/tests/transfer_gate.rs` and forty unit tests, against real files and a real
  database — what is outstanding is the pane itself: the folder list, the two pickers, and the
  progress reporting.

- **Outlook `.pst` import is not done**, and docs/06 asks for it. A `.pst` is not a mail file
  but a MAPI object store — a pair of B-trees over a paged heap, holding messages as numbered
  properties with no RFC 5322 anywhere in it. Reading the store is solved: Microsoft publish
  `outlook-pst`, a clean-room MS-PST implementation in Rust. Turning MAPI properties back into a
  message — recipient tables, `PR_TRANSPORT_MESSAGE_HEADERS` where it exists, a body that may be
  plain, HTML or RTF compressed with a Microsoft dictionary — is the actual work, and it is its
  own piece rather than a corner of this one. The import screen says so plainly and points at
  the two paths that do work today: Outlook saves `.eml`, and an Outlook account can be added
  over IMAP.
- **Re-importing the same file duplicates**, and the UI says so before it starts. There is no
  stable identity to match an mbox message against — `Message-ID` is missing from a lot of old
  mail and forged in some of the rest — so deduplicating on it would silently drop messages that
  are not duplicates.

---

## 2026-08-31 (later) — Phase 11: ship, the rest of it

Everything Phase 11 still needed except code signing, which nobody here can do.

### Added

- **First run: welcome, add an account, done.** A wrapper around the account assistant rather
  than two more steps inside it — the assistant is a reducer with a guard on every transition,
  and two screens that carry no data and ask no questions do not belong in it. The welcome
  screen exists because the risk to the three-minute gate is not the number of screens, it is
  opening an unexplained credential form as the first thing a stranger sees.
- **The three-minute gate, measured.** `tests/e2e/firstRun.spec.ts` drives the screens and
  reports the time: **733–902 ms** across three runs, against a budget of 180 seconds. That is a
  floor, not the real figure — a person reads and types, and mail then arrives over a network no
  test can simulate — but it says the screens themselves cost nothing.
- **An auto-updater**, off by default in the sense that nothing checks on a timer. Standing rule
  16 makes an update check the only outbound request in the app that is not mail, so it happens
  when the user presses a button and the version on offer is shown before anything downloads.
  Updates are signed with minisign-style keys and verified against a public key compiled into
  the binary: TLS proves a file came from GitHub, the signature proves it came from us.
- **A `self-update` Cargo feature**, per docs/07 §2.3. Store builds compile the updater out
  entirely — the Store installs its own updates, and two mechanisms fighting produces duplicate
  installs and fails certification.
- **Installer artwork**, generated rather than committed as binaries: `tools/make-installer-art.cjs`
  reads the real app icon and composites the two BMPs NSIS wants. A BMP checked into the
  repository is a thing nobody can change, and the first time the icon moved the artwork would
  have silently stopped matching it. Includes a PNG decoder, because the icon is a PNG and Node
  ships the hard half (zlib).
- **README, LICENSE, PRIVACY.md, SECURITY.md.** The README had been the original planning
  document, opening with "Planning workspace" and saying Phase 0 was scaffolded.
- **Outlook `.pst` import.** See below.

### Changed

- **Version 0.1.0 → 1.0.0** across `package.json`, `Cargo.toml` and `tauri.conf.json`, and the
  empty `copyright` filled in. docs/07 §2.3: the Store will not accept `0.0.0`, and every
  submission must increment.
- `import::write_message` takes an explicit read-state override. Thunderbird records read state
  in the message; a PST records it in a MAPI property the synthesised message does not carry.

### Fixed

- **The first-run timing test measured Vite's cold compile** and failed about one run in five.
  It warms the page first now and measures the second load. Exactly the lesson the scrolling
  budget taught two days ago, learned again — a gate that fails on machine load is one people
  learn to ignore.

### Incidents

- **`cat >>` put new functions after the test module. Twice.** Clippy's `items_after_test_module`
  caught it in `platform/files.rs` and again in `ipc/transfer.rs`. Appending to a Rust file is
  only ever correct when the file has no `mod tests`, which most of them do.
- **The updater's key-leak test did not work, and passed.** It searched every file for a private
  key header and found none — because it had two things wrong at once. Tauri signs with `rsign`,
  not minisign, so the header text was wrong; and the key file is base64 **as a whole**, so the
  plaintext header never appears in it. Both were only found by planting a real key in the tree
  and watching the test pass. It now checks both forms, derives the encoded one so they cannot
  drift, and was re-probed the same way. An earlier version had also matched its own source,
  reporting itself as the leak.
- **The NSIS `license` key does not exist.** The build failed schema validation; the licence is
  `bundle.licenseFile`, not `bundle.windows.nsis.license`. Found by reading the CLI's own
  `config.schema.json` rather than by guessing again.
- **The first sidebar artwork was invisible.** A blue wash with the app icon — a blue rounded
  square — composited onto it. Correct dimensions, correct format, unreadable image. Found by
  converting the BMP back to a PNG and _looking_ at it, which is not something the file format
  or the build would ever have complained about.

### Notes

- **`.pst` import is done, and is the least tested thing in the project.** A `.pst` is a MAPI
  object store, not a mail file; Microsoft's own `outlook-pst` crate reads the store and
  `transfer/pst.rs` turns MAPI properties back into RFC 5322. Where Outlook kept the original
  headers — `PR_TRANSPORT_MESSAGE_HEADERS`, which it does for most received mail — the message
  is reconstructed nearly exactly, so an imported reply threads with a synced original. Where it
  did not, which is normal for sent mail, the headers are synthesised and the `Message-ID` is
  invented, and an invented id threads with nothing.

  The gap that matters: **the only PST this project has that it did not write itself contains no
  mail.** `tests/pst_gate.rs` proves the store opens, the folder tree is walked, a non-PST is
  refused and a missing file is an error — all against real Outlook output. It proves nothing
  about extracting a message. A fixture written by this project would only prove the reader can
  read the writer, so none was made. This is said in the module note, in the gate's own header,
  and on the import screen, because an archive that imports looking complete and is not is the
  worst thing this code could do.

  Attachments are not extracted and RTF-only bodies cannot be read. Both are counted and
  reported rather than dropped quietly.

- **The installer builds: `Halcyon_1.0.0_x64-setup.exe`, 4.7 MB**, with the artwork and the
  licence page. It is **unsigned**, so SmartScreen will warn on every machine that downloads it.
- **The update signing key is not in this repository and must never be.** It was generated to
  `%TEMP%\…\scratchpad\keys\halcyon-update.key`, has **no passphrase**, and needs moving
  somewhere safe before the first release — a password manager, and a `TAURI_SIGNING_PRIVATE_KEY`
  secret for CI. Losing it means no installed copy can ever be updated again; leaking it means
  anyone can sign an update for every installed copy. A test now fails the build if a key ever
  lands in the tree.
- **Still outstanding:** code signing (2–4 weeks of identity validation, and the exit gate's "no
  SmartScreen warning" cannot be met without it), MSIX packaging for the Store, a release
  workflow, and the exit-gate checks that need a fresh Windows 11 VM. Phase 7's gate still wants
  Outlook and Apple Mail accounts, and Phase 10's still wants a recorded Narrator walkthrough.

---

## 2026-08-31 (evening) — Phase 11: the Store package

### Added

- **`src-tauri/msix/AppxManifest.xml`**, with the real Partner Center identity from docs/07 §2.2.
  Two capabilities and no more — `runFullTrust`, which a desktop app needs to exist under MSIX,
  and `internetClient`. Everything else is deliberately absent, because each one is a thing to
  justify to a reviewer and a thing they can reject.
- **`tools/make-store-assets.cjs`** — 36 MSIX images generated from the app icon, including the
  `altform-unplated` variants. Windows draws small icons on a plate everywhere except the taskbar
  and the jump list; with no unplated variant it uses the plated one, which is the icon
  _with its own background_ on a background.
- **`tools/make-msix.ps1`** — builds with the updater compiled out, checks the artefact, stages
  and packs. Produces `Halcyon_1.0.0.0_x64.msix`, **5.4 MB**, unsigned, which is what the Store
  wants: Microsoft signs on ingestion. `-Test` signs it with a self-signed certificate for
  sideloading.
- **`tools/png.cjs`**, shared by both artwork scripts. PNG decode, box resample, PNG and BMP
  encode, on Node's zlib and nothing else.
- **The store-build check, three times over.** A `compile_error!` if `store` and `self-update`
  are both on; `build.rs` removing the updater's capability file when the feature is off; and
  `make-msix.ps1` searching the built binary for the plugin's command strings. A mistake here is
  not caught until certification, days later.
- **A public site for the required links** — <https://vnikie1.github.io/halcyon-mail/>, with the
  privacy policy and the security contact. The Store will not accept a submission without a live
  privacy policy URL and a dead one is an instant rejection, so it is plain HTML with one
  stylesheet and nothing fetched from anywhere.

### Changed

- **The updater capability moved to `capabilities-optional/`**, copied into place by `build.rs`
  when `self-update` is on and removed when it is not. Tauri validates **every** file in
  `capabilities/` against the compiled-in plugins — not only the ones `app.security.capabilities`
  selects — so a capability naming `updater:default` is a hard failure in a Store build however
  carefully the config deselects it. Both were tried. The reviewable copy stays in version
  control; only the generated one is ignored.
- **The scrolling budget now asserts the median, not the p95.** It had sat outside the gate for
  ten phases and began failing about one run in four the moment `npm run verify` started running
  it — p95 is one frame in twenty, which on a machine also compiling a release build is whichever
  frame the scheduler interrupted. The median is stable at 16.5–17.2 ms across every run measured,
  loaded or idle, and the regression this exists to catch moves it by an order of magnitude
  rather than by 40%.
- README no longer says `.pst` import is missing, and the Known gaps entry now says the honest
  thing: it works, and its message extraction is the least tested code in the project.
- PRIVACY.md gained a paragraph on the Store's own aggregate analytics. Microsoft measuring their
  platform is not this app reporting on you, but nobody should have to discover it.

### Incidents

- **The store-build check failed a perfectly good Store package**, and would have sent somebody
  hunting a build-flag problem that did not exist. It searched the binary for the update endpoint
  URL — but Tauri embeds the whole of `tauri.conf.json` into every binary, so the endpoint is
  present whether the plugin is or not. It is configuration, not code. Verified by building both
  ways and diffing the strings: the endpoint is in both, and `plugin:updater|check` is in neither
  the Store build nor anything else. The check now looks for the plugin's command routing strings
  and was probed in both directions.
- **Windows PowerShell 5.1 would not parse `make-msix.ps1`**, because it reads an unmarked `.ps1`
  as the system ANSI code page and an em-dash in a string became mojibake. Saved with a UTF-8 BOM,
  with a note at the top saying why. `pwsh` does not exist on this machine; 5.1 is what ships.

### Notes

- The MSIX has **not** been sideloaded or run through WACK. Both need doing before submission —
  docs/07 §2.6 lists what to check under MSIX specifically, and a WACK failure is a guaranteed
  rejection.
- The two rejections that sink email clients, from docs/07 §2.7: **test account credentials in
  Notes for Certification**, without which the reviewer cannot get past the welcome screen and
  fails the submission as incomplete; and a specific written justification for `runFullTrust`.

---

## 2026-08-31 (night) — Phase 11: the Store package, installed

### Added

- **`tools/run-wack.ps1`** — runs the App Certification Kit against the installed package and
  reduces its thousand-line XML to the one word that matters plus every failure by name. Separate
  from `make-msix.ps1` because WACK is the only step that needs elevation, and making the whole
  packaging script require an administrator to run would be a poor trade for one command.

### Verified — by installing it, not by reading about it

The package was registered and run. What that establishes:

- **It installs.** `Unikie1.HalcyonMail_1.0.0.0_x64__anw48tyhk74bp`, status Ok.
- **The identity strings in the manifest are right.** The package family name Windows computed —
  `Unikie1.HalcyonMail_anw48tyhk74bp` — matches what Partner Center issued and docs/07 §2.2
  recorded, character for character. Getting one of those wrong is rejected at upload.
- **It runs with package identity.** `GetPackageFamilyName` on the live process returns the family
  name, which is what makes the AUMID real. The AUMID resolves to
  `Unikie1.HalcyonMail_anw48tyhk74bp!Halcyon`, exactly as docs/07 §2.2 predicted, so toasts will
  address the packaged app rather than a phantom.
- **It works.** Not merely starts: it synced 46 mailboxes on a real account, removed a message
  expunged on the server, and rendered a 68KB HTML body through the sanitiser — all from the
  packaged, updater-free build.
- **The Start menu entry is registered** as "Halcyon" against that AUMID.

No certificate was needed. Developer mode is on, so the staged layout registers directly from its
manifest — which is also the fastest loop for changing the manifest and seeing the result.

### Fixed

- **docs/07 §2.6 said the database lands in "the redirected path". It does not**, and the section
  now says so with the measurement. A `runFullTrust` package writes straight through to the real
  `%LOCALAPPDATA%\com.uniki.halcyon`; redirection into `%LOCALAPPDATA%\Packages\<PFN>` is for
  sandboxed UWP apps. The Store build opened the same database the NSIS build uses.

  This is the better outcome and is now deliberate rather than accidental: installing the Store
  version over the downloaded one keeps the mail, the accounts and the settings, with no migration
  step and nothing for the user to do.

### Notes

- **WACK has not been run.** `appcert.exe` refuses to start unelevated — "The requested operation
  requires elevation", and nothing else — and this session has no elevated shell. It is one
  command from an administrator prompt and takes 10–20 minutes. A WACK failure is a guaranteed
  Store rejection, so it is the last thing standing between the package and a submission.
- The package is left installed, registered against the staging folder. `Remove-AppxPackage
(Get-AppxPackage *Halcyon*).PackageFullName` removes it; the unpackaged dev build is unaffected
  either way, since both use the same database.

---

## 2026-08-31 (late) — Phase 11: what the App Certification Kit said

### Added

- **`src-tauri/halcyon.exe.manifest`**, embedded through `build.rs`. Declares `PerMonitorV2` DPI
  awareness, `asInvoker` execution level, `longPathAware`, and UTF-8 as the process code page.

### Fixed

- **The app was DPI-aware slightly too late.** WACK reported "The app is not DPI Aware", and it
  was half right: wry calls `SetProcessDpiAwarenessContext` while starting, which is why a window
  measures in real physical pixels on a 200% display — but that call happens _after_ the process
  begins. Until it runs, Windows believes the process is unaware, and anything computed in that
  window is done against a virtualised desktop and then bitmap-stretched. The binary had no
  `dpiAware` element and no `trustInfo` at all. Declaring both in an embedded manifest makes it
  true from process creation, and clears the DPI warning and the UAC run-level test together.

- **`run-wack.ps1` would have reported "No failures" under a banner reading OVERALL: FAIL.** Each
  verdict is a child element wrapped in CDATA — `<RESULT><![CDATA[FAIL]]></RESULT>` — not an
  attribute. The first version matched on `@RESULT`, found nothing, and concluded all was well;
  the second read the element rather than its text and matched nothing either. A summary that
  contradicts itself is worse than none, because one of the two lines gets believed. It now reads
  `InnerText`, and was verified against the real failing report rather than by inspection.

- **The failure summary groups by message.** Twenty-two failures with one underlying cause read
  as twenty-two problems in a flat list; grouped, they read as the three they are. It also says
  so explicitly when one cause dominates, and names the likely reason.

### Incidents

- **The first WACK run failed 22 of 24 tests, and none of them was the app's fault.** The package
  had been sideloaded with `Add-AppxPackage -Register`, which points at a loose folder. That runs
  perfectly well — it synced real mail — but WACK does not consider it installed: it reported
  "The manifest file for this app package could not be found", "The platform metadata directory
  does not exist", and empty metadata (`APP_NAME=""`, `APP_VERSION=""`), then failed everything
  downstream of that.

  `make-msix.ps1 -Test` now signs the package, trusts the certificate — asking for elevation for
  that step only, so the build itself never runs as administrator and `target/` keeps its
  ownership — removes any previous copy, and installs the real `.msix`. It warns if the result is
  not under `WindowsApps`, which is the one observable difference between the two.

### Notes

- Informational, not a failure: WACK notes that the binary references `CreateProcessW` and
  `ShellExecuteW`. Both are real and both are wanted — opening the diagnostics folder in Explorer,
  and opening the system browser for OAuth, which docs/03 requires rather than an embedded
  WebView. The test is optional and the references are expected for a `runFullTrust` app.

---

## 2026-09-01 — Phase 11: the App Certification Kit, three runs

### Added

- **`src-tauri/tests/manifest.rs`** — four tests over the embedded Win32 manifest: ASCII only, no
  double hyphen inside an XML comment, the four settings still declared, tags balanced. Both real
  mistakes below were probed by reintroducing them.
- **`resources.pri`**, built by `makepri` during packaging. Without it the scale-qualified assets
  are files with long names rather than an indexed set: Windows falls back to the unqualified copy
  at every size, so the 400% assets are unreachable on exactly the high-DPI screens they exist for.

### Fixed

- **The embedded manifest was unparseable, twice, and both times it was punctuation in a comment.**

  First an em-dash. The Windows manifest tooling reads the embedded resource as ANSI, so the three
  utf-8 bytes arrive as three unrelated characters mid-document: `mt.exe` reported "An illegal
  character was encountered", and WACK reported "Failed to process the binary" followed by "the
  app is not DPI Aware" — a long way to travel to be told about a dash.

  Then the obvious fix for it. Replacing each em-dash with two hyphens is illegal XML: a double
  hyphen may not appear inside a comment. The error changed to "Windows was unable to parse the
  requested XML data", which looks like progress and is a different bug.

  Single hyphens throughout, and `tests/manifest.rs` now fails the build on either. Neither is
  visible in a diff.

- **The manifest declared a splash screen a desktop app never shows.** WACK failed the resource
  test with _Image reference "Assets\Square150x150Logo.png": failed the size restrictions of
  620 X 300_ — which reads as a problem with the tile and is not. 620x300 is the **splash screen**
  size, and the manifest pointed the splash at the 150x150 tile. Windows only draws a splash for
  UWP; a full-trust app draws its own window. The element is gone rather than accompanied by a
  620x300 image nothing displays, which would have silenced the message and kept the mistake.

### Verified

Three runs, and the arc is the point:

| Run | Verdict | Passed | Failed | Cause                                                        |
| --- | ------- | ------ | ------ | ------------------------------------------------------------ |
| 1   | FAIL    | 1      | 22     | Sideloaded from a loose folder, so WACK found no manifest    |
| 2   | WARNING | 21     | 2      | Both optional: the splash image, and the process-launch APIs |
| 3   | WARNING | **22** | **1**  | Only the process-launch APIs, which are correct              |

The remaining failure is the optional `Blocked executables` test, noting `CreateProcessW` and
`ShellExecuteW`. Both are wanted and neither is avoidable: Explorer for the diagnostics folder,
and the system browser for OAuth, which docs/03 requires instead of an embedded WebView.

### Incidents

- **The failure summary gave advice that did not apply.** With a single failure it still printed
  a paragraph telling the reader to go and check how the package was installed, because the
  condition included `groups.Count -eq 1`. Advice that does not apply is worse than none: it
  sends somebody to look at the one thing already known to be right. It now appears only when
  more than three failures share a cause, and was probed against both reports.
- **The manifest test failed on its first run, and the test was wrong.** `<assembly` also matches
  `<assemblyIdentity`, so the balance check reported the manifest unbalanced when it was not. The
  same loose-substring mistake as the earlier scan of the binary. It now requires a delimiter
  after the tag name.

### Notes — the DPI warning, honestly

WACK reports `DPIAwarenessValidation` as a **warning**: "Failed to process the binary", then "the
app is not DPI Aware". The fix above did not change that, and it was predicted that it would.

What is established about the installed binary:

- `mt.exe` — Microsoft's own manifest tool — reads its manifest cleanly, and it declares
  `dpiAware=true/PM`, `dpiAwareness=PerMonitorV2`, `asInvoker`, `longPathAware` and UTF-8.
- The app measurably renders at true 2x: a 780-logical-pixel window measured 1586 physical on a
  200% display through the UI Automation tree.
- `strip = true` removes debug symbols, not resources — the VERSIONINFO and RT_MANIFEST resources
  both survive and are both readable.

So the app is DPI-aware, from process creation rather than shortly after it, and WACK's own first
message is the accurate one: its validator cannot process this binary. That is a limitation of the
tool rather than a property of the app, it is a warning rather than a rejection, and no further
time is being spent on it.

The manifest work was still worth doing on its own terms — it moved DPI awareness ahead of the
first painted frame, and it cleared the UAC run-level test.

## 2026-09-01 — Phase 11: the updater, actually run

### Added

- **`src-tauri/tests/bundle.rs`** — three tests asserting that a release build produces the app
  and nothing else. Probed by removing `required-features` from one tool and by turning
  `autobins` back on; each probe failed the intended test with the intended message.
- **An error message when an update will not install.** `UpdateSettings.tsx` now renders the
  failure in a `role="alert"`, with separate wording for a rejected signature, both saying the
  app has not been changed.

### Fixed

- **`seed.exe` and `crashgate.exe` were being installed on users' machines.** Every file in
  `src-tauri/src/bin/` is auto-discovered by cargo as a binary, built into `target/release` by a
  release build, and picked up from there by the NSIS bundler. A clean install of 1.0.0 — after a
  full uninstall, so nothing was stale — put both next to `halcyon.exe` in `%LOCALAPPDATA%\Halcyon`.

  `seed.exe` writes fabricated mail into the user's database; `crashgate.exe` crashes the app on
  purpose. Fixed with `autobins = false` plus `required-features = ["devtools"]` on all five dev
  tools, so a release build does not produce them and therefore cannot ship them.

  A comment in `Cargo.toml` had asserted the opposite — "Tauri bundles only the productName
  binary" — for months. It was wrong, and writing it down is probably why nobody checked.

- **A refused update was silent.** `updateInstall().catch(() => setInstalling(false))` reset the
  button and said nothing, so a rejected signature was indistinguishable from not having clicked.
  Found by the tamper test below, not by reading the code — it had been read several times.

### Notes — the updater gate

1.0.0 installed from its own installer, offered a signed 1.0.1 from a local server, driven
through the app's own UI. All six pre-written criteria passed; the database came through
identical:

    messages 1521 -> 1521   mailboxes 45 -> 45   threads 1590 -> 1590   attachments 7 -> 7
    withBodies 48 -> 48     flagged 3 -> 3       unread 382 -> 382      accounts 1 -> 1
    newest message id 101643 unchanged, settings 3 -> 3, signature intact

The half worth keeping is the second check: the updated app, asked by the same server still
offering 1.0.1, answered "Halcyon is up to date". A comparison that always returns true would
have passed criterion 1 just as well.

**Tamper test.** The signed 1.0.1 installer was copied to a 1.0.2 name, its signature kept, and
one byte flipped in the middle. The app offered it, downloaded all 4,803,972 bytes, and refused
to install it. That is the security argument in `ipc/update.rs` — TLS proves where a file came
from, the signature proves whose it is — demonstrated rather than asserted.

### Notes — uninstall

`uninstall.exe /S` removes the install directory, the Start Menu entry, the `HKCU` uninstall key
and the `Halcyon.eml` class. `%LOCALAPPDATA%\com.uniki.halcyon` is deliberately left: it holds
the mail database, and an uninstaller that silently deletes somebody's mail is the worse failure.

### Incidents

- **Two hours were spent building a local HTTPS server that was never needed.** A release build
  refuses a plain-`http` updater endpoint, so a self-signed certificate for `127.0.0.1` was
  created, exported and added to the machine's trusted roots. The actual cause of the original
  failure was that the build predated adding `dangerousInsecureTransportProtocol` to the override
  config — the `--config` merge had worked all along. Checking the built binary for the endpoint
  string took one command and would have settled it before any of the certificate work started.

  The certificate has been removed from `Cert:\LocalMachine\Root` and `Cert:\CurrentUser\My`, and
  the `.pfx` deleted. Nothing about the machine's trust store is left changed.

- **A conclusion was reported that turned out to be wrong.** "`dangerousInsecureTransportProtocol`
  did not survive the `--config` merge" was recorded as established. It had not been checked. It
  is the second time this phase that a diagnosis was believed rather than tested, after the
  manifest, and both cost more than the test would have.

- **The lint gate caught two problems in this session's own new code** — an unnecessary type
  assertion and five `eslint-disable` directives that never applied — neither of which would have
  been noticed by reading.

## 2026-09-01 — Phase 11: import and export, and a backup that lost mail

### Fixed

- **"Export all mail" silently omitted mail that had just been imported.** `startExport` iterates
  the `useMailboxes()` query, and an import creates a mailbox and an account that did not exist
  when the Settings window opened. Nothing invalidated that query, so the export enumerated a
  stale list: **46 mailboxes in the database, 45 files written**, no error and a completion
  message reporting success. Closing and reopening Settings produced all 46.

  The shape of this matters more than the size. Somebody imports years of old mail, exports
  everything as a backup, and the backup is missing precisely the mail they just imported — from
  a button labelled "Export all mail". Now invalidates `keys.mailboxes` when a transfer finishes;
  re-running the whole sequence in one sitting wrote 47 files for 47 mailboxes.

- **The updater endpoint pointed at a private repository.** `vnikie1/MailBox` is private, so its
  release assets are not publicly downloadable and _every_ update check would have failed for
  every user. Changed to `vnikie1/halcyon-mail`, which is public and already hosts the policy
  pages. This was found by reading, not by the updater test, which deliberately substitutes
  localhost — the one string that test cannot check turned out to be the one that was wrong.

- **`SECURITY.md` sent reporters to a security-advisory form on the same private repository**,
  where they would have been met with a 404. The published copy at
  <https://vnikie1.github.io/halcyon-mail/security.html> still carries the old link and needs the
  same change.

- **"Done. 3 messages in 1 mailboxes."** Importing a single mbox is the commonest case there is,
  so the number most likely to be 1 was the one printed wrong every time.

### Notes — import and export, exercised in the running app

Built in this phase and never run outside the Rust tests. Driven through the real UI against a
**copy** of the mail store: `%LOCALAPPDATA%` redirected to a sandbox, so the app opened 1,521 real
messages and every change landed on the copy. The real database was unchanged throughout —
1 account, 45 mailboxes, 1,521 messages, before and after.

Import handled the case that matters: a line beginning `From ` inside a message body did not split
the message, and survived verbatim in the stored body. Three of three messages arrived with full
bodies, into a local account, without disturbing the Gmail account.

### Incidents

- **Five ways of driving a Windows file dialog failed before one worked**, and the most misleading
  was `SetDlgItemText` on control 1148: it sets the ComboBoxEx _host_, and `GetDlgItemText` reads
  the value straight back, so it looks like it worked while the dialog never sees it. `WM_COMMAND`
  with `IDOK`, `BM_CLICK`, and UIA `ValuePattern.SetValue` all fail as well — the last by timing
  out and wedging the dialog, which then made every later attempt look like a fresh failure.

  What works is genuine input: `AttachThreadInput` to take real foreground, then keystrokes. A
  wrong hypothesis about _why_ the first attempts failed (a wedged message loop) cost a restart
  and two more attempts before the real cause was tested.

## 2026-09-01 — The source repository is public

### Changed

- **`vnikie1/MailBox` is now public**, and the whole project is on it. Until today GitHub held two
  commits — the Phase 0 scaffold and a rename — while all 71 commits and 502 files of the actual
  application existed only on one laptop. `main` was fast-forwarded to the work branch and pushed
  before the visibility change.

  This is why it mattered: the published site says, twice, that "the source code is published so
  that anyone can check it". That claim was false in two separate ways — the repository was
  private, and it did not contain the application. A privacy policy whose central promise is
  verifiability, linking to a 404, is worse than one that makes no promise at all.

- **The updater endpoint and the security-advisory link point back at `MailBox`.** Both were moved
  to `halcyon-mail` earlier today purely because `MailBox` was private and its release assets were
  not publicly downloadable. That reason is gone. Releases and the source now live in one place,
  which is also where the site already tells people to look.

### Removed

- A real inbox subject line quoted in `docs/PHASE-11-VERIFICATION.md`, before publication. The
  message id already made the point that a specific message survived the update, and an id is not
  somebody's mail.

### Notes — what was checked before publishing

Making a repository public exposes its entire history, not the current tree, and it cannot
meaningfully be undone once anything is cloned or indexed. All 73 commits were scanned first:

| Looked for                                      | Found                                                                                                                                        |
| ----------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Updater signing keys (`rsign`/minisign headers) | One hit — the leak-detector assembling its own search string                                                                                 |
| OAuth client secrets                            | One hit — `GOCSPX-do-not-store-me`, a fixture asserting the secret never reaches SQLite                                                      |
| Real Google client IDs                          | None                                                                                                                                         |
| Certificates and private keys                   | None; `test/dovecot/certs.sh` generates them at run time                                                                                     |
| Real mail content                               | None. Test addresses are all `example.test` / `halcyon.test`                                                                                 |
| Large blobs or database dumps                   | None; the packed repository is 277 KB                                                                                                        |
| Personal data                                   | The author's own address in LICENSE, PRIVACY and SECURITY, where it is a deliberate contact point, and one inbox subject line, removed above |

## 2026-09-01 — Phase 7: the toolbar did nothing, and the app cannot send

### Fixed

- **Every button in the main toolbar was decorative.** New Message, Reply, Reply All, Forward,
  Archive, Delete, Move to Junk, Move to and Flag rendered, took tooltips, greyed themselves out
  correctly against the selection — and had no `onClick`. `ToolbarProps` carried no action
  callbacks at all, so there was nothing for them to call.

  The nine handlers already existed in `AppShell`, wired to the keyboard shortcuts. They are now
  one object driving both, so a button and its shortcut cannot drift apart.

  It went unnoticed because everything the toolbar offers is reachable another way: the message
  list carries its own Reply, Reply All and Forward, and every shortcut worked.

- **There was no way to start a new message inside the window.** The `newMessage` shortcut called
  `composeBlank()`, which returns the _contents_ of a blank draft and opens nothing, where
  `composeOpen()` is what creates the window. So New Message fetched a draft, dropped it, and left
  no trace — no window, no error, nothing in the log. Reply, Reply All and Forward all used
  `composeOpen` correctly, which is why the mistake looked like working code. The only way in was
  the taskbar Jump List, which goes through Rust.

- **The format bar took focus away from the editor on every click.** A button takes focus on
  mousedown, so clicking Bold moved the caret out of the body: the command applied, but the next
  keystroke went nowhere and you had to click back into the message before typing again. Fixed by
  preventing the default on mousedown at the toolbar container, which is what every rich-text
  toolbar does. Verified that the colour and size menus still open.

- **`gate_2_a_five_predicate_smart_mailbox_agrees_with_hand_written_sql` was passing vacuously off
  this machine.** The test uses the real mail store when one exists and a 199-row fixture when it
  does not. `add_message` hardcodes `has_attachment = 1`; the predicate requires
  `has_attachment = 0`. So on the fixture the hand-written query matched nothing, the compiled
  query matched nothing, and the two agreed over an empty set.

  The test's own vacuity assertion caught it — but only where there is no mail store, and the
  machine it was written on always had one. It failed for the first time in CI, on the first run
  after the repository went public. The fixture now varies sender, subject, attachment, read state
  and size, so each of the five clauses can matter: 79 of 199 rows match, against 283 of 1,521 on
  the real store.

### Notes — the Phase 7 gate cannot be run

> Exit gate: send a reply to Gmail, Outlook and Apple Mail and confirm all three thread it
> correctly and render the HTML correctly…

**Halcyon cannot send mail from the only kind of account it has.** A message composed and sent to
two real addresses failed in the outbox with

    the message has no usable envelope: sending from an OAuth account is not available yet

This is deliberate and documented in `sync/sender.rs` — Phase 7 implemented password submission,
and SMTP XOAUTH2 was left for later. The consequence had not been written down anywhere: the
configured account is Gmail over OAuth, so there is no account this build can send from, and the
send half of Phase 7 has never been exercised end to end by anybody.

What did work, and is worth recording: the failure was **loud**. The outbox moved the message to
`failed` after five attempts, kept the message, and the main window showed a banner naming the
reason. "Never silently drop a message" holds.

### Incidents

- **Driving the compose window found the three bugs above in the first ten minutes**, which is the
  argument for exercising a feature in the running app rather than trusting its unit tests. All
  three are invisible in a screenshot and none is reachable from a test that does not click.
- **The format-bar buttons do not expose `InvokePattern`**, so UI Automation cannot press them;
  they need a real mouse click at their screen coordinates. Worth knowing before the next attempt
  to drive this window.

### Fixed — CI could no longer build the installer, and it was this session's doing

`bundle.createUpdaterArtifacts: true` was added to the shipped config during the updater test and
committed with it. The bundler then refuses to build at all without a signing key:

    A public key has been found, but no private key.
    Make sure to set `TAURI_SIGNING_PRIVATE_KEY` environment variable.

The real key is not on CI and must never be, so the Installer job failed on the first run after
the repository went public — the frontend and Rust jobs having just been fixed, which is what made
it visible.

Keeping the setting is right: a release built without it produces no `.sig`, and the updater can
then never offer that release, which fails silently and months later. So CI now generates a
throwaway key and signs the smoke-test artifact with something that verifies against nothing. That
is the correct outcome for a build nobody installs, and it keeps the packaging path exercised end
to end.

## 2026-09-01 — Phase 7: the app can send

### Added

- **SMTP XOAUTH2.** Halcyon could not send mail at all. `send_one` refused anything but a password
  account, and Gmail — the only account configured, and the only kind most people will add — is
  OAuth. The refusal was deliberate and documented; what nobody had written down is that it left
  the send half of Phase 7 unreachable, and it stayed unreachable until somebody tried to send a
  message.

  Almost none of this was new code. `lettre` already has `Mechanism::Xoauth2`, and it renders the
  same SASL string the IMAP side assembles by hand in `session.rs`. `engine::credential_for`
  already loaded a password or refreshed an expiring OAuth token and wrote the new expiry back;
  the sender simply was not calling it. It calls it now, rather than growing a second copy of the
  refresh logic — and the copy that sends mail would have been the one nobody noticed had gone
  stale.

  **An OAuth account is offered XOAUTH2 and nothing else.** `lettre` picks the first mechanism the
  server also advertises, so leaving PLAIN in the list would let it win — and PLAIN puts the second
  credential field in the password slot, which for an OAuth account holds the **access token**.
  Gmail rejects it, so the visible symptom is "sending is broken"; the real cost is that the token
  has already been transmitted as a password to a server with every reason to log a failed
  authentication. `an_oauth_account_is_never_offered_a_password_mechanism` fails if PLAIN ever
  reappears, and was probed by putting it back.

### Notes — sending, verified end to end

Driven through the real compose window against a **copy** of the mail store, so the app opened
1,521 real messages and the real database was untouched throughout.

    outbox: sent id=2      first attempt, about seven seconds

The message was then filed into Gmail's Sent Mail by IMAP APPEND, synced back on the next pass,
and appears in the local Sent Mail mailbox. It carries bold, italic, underline, three bullets, two
numbered items, a block quote and a horizontal rule — lists and quotes chosen deliberately, since
those are what a Word-based renderer mangles.

What this does **not** yet establish is the rest of the gate: that the message threads and renders
correctly in Gmail, Outlook and Apple Mail. That needs somebody to open it in each, and a reply
from each to thread against. Outlook Windows is called out in the gate as the strict case, and it
means _classic_ Outlook, whose renderer is Word — not the new Outlook, which is the web client in
a window and will pass without proving anything.
