# Phase 1 — verification record

Roadmap gate (`docs/04-roadmap.md`): *gallery renders all primitives; zero hardcoded
colours/sizes anywhere (enforce with a stylelint rule); visual diff of the gallery is
stable; keyboard nav works on every interactive primitive.*

Prompt-library gate (`docs/06-prompt-library.md` § Phase 1): *gallery shows all primitives
in all states in both themes; stylelint passes; Vitest covers keyboard behaviour for Menu,
TokenField and Popover; a Playwright screenshot of the gallery is committed as the visual
baseline.*

**Status: passed.** Every clause is verified below. Two things are recorded as open rather
than done: the 320ms `--dur-sheet` conflict with standing rule 7 (§4.1), which needs your
decision, and display scaling above 100 %, which cannot be exercised from Chromium (§5).

---

## 1. Verified

| Check | Command | Result |
|---|---|---|
| Formatting | `npm run format:check` | pass |
| Lint | `npm run lint` | 0 problems |
| Design-token rule | `npm run lint:css` | 0 problems |
| Types | `npm run typecheck` | pass |
| Unit tests | `npm run test` | 45 / 45 (was 13) |
| End-to-end | `npx playwright test` | 11 / 11 (was 6) |
| Rust formatting | `npm run rust:fmt` | pass |
| Rust lint | `npm run rust:clippy` | 0 warnings |
| Rust tests | `npm run rust:test` | 4 / 4 |
| Production build | `npx vite build` | 218 kB JS, 15.3 kB CSS, 7 woff2 subsets |
| Tauri window | `npm run app:dev` | launches, `system backdrop applied effective=MicaAlt`, zero warnings or errors in the log |

`npm run verify` is green end to end.

### Gate clause by clause

- **All sixteen primitives, every state, both themes.** `/dev/gallery` renders its
  specimens twice, under `[data-theme='light']` and `[data-theme='dark']` subtrees. The
  e2e test asserts a section per primitive exists in *both* columns, so a primitive added
  without a specimen fails the build rather than being forgotten.
- **Zero hardcoded values.** The stylelint rule written in Phase 0 covers every
  `*.module.css`; nineteen new modules pass it. Values that CSS cannot hold — Floating
  UI's numeric offsets, unmount delays — are read back out of the cascade by
  `src/lib/tokens.ts` rather than written as literals in TypeScript. See §3.
- **Stable visual diff.** `tests/e2e/gallery.spec.ts-snapshots/gallery-chromium-win32.png`
  is committed, 1400 × 3022, covering all sixteen primitives in both themes.
- **Keyboard nav on every interactive primitive.** 24 Vitest cases across Menu (8),
  TokenField (10) and Popover (6). Run three times consecutively to confirm they are not
  order-dependent — see the incident in §6.

---

## 2. What was built

**Tokens.** `component.css` now covers all of `docs/02` §6.2–§6.10 and the three density
modes of §7. `primitive.css` gained the material blur radii, the shimmer duration and the
interaction constants; `semantic.css` gained the material filters, the scrim, the scrollbar
thumb, the destructive role and the focus ring. Additions beyond the doc's literal token
list are itemised in §4.2.

**Primitives**, all in `src/ui/`, one CSS Module each: Button, IconButton, Menu,
ContextMenu, Popover, Tooltip, TextField, TokenField, Chip, Avatar, Badge, Divider, Sheet,
Toast, Skeleton, ScrollArea.

**Theming.** `[data-theme]`, `[data-density]` and `[data-reduce-transparency]` are written
by exactly one function, `applyAppearance`, which resolves what Windows reports against
what the user pinned. Preferences live in `src/store/settings.ts`, persisted to
localStorage and read synchronously at boot so the pre-paint frame cannot disagree with
the frame after it. `[data-window-inactive]` remains the single token remap from Phase 0.

**Font.** Inter is now real. `@fontsource-variable/inter`, self-hosted per `docs/02` §2 —
Vite fingerprints and bundles the woff2 files, nothing is fetched at runtime, and the build
works offline. See §4.3 for why the `opsz` cut.

---

## 3. Reading tokens from TypeScript

Two things need a token's *value* in JavaScript, and both would otherwise smuggle design
values past the stylelint rule:

- **Floating UI offsets** are numbers. A menu sitting 4px from its trigger would carry a
  literal `4` in a component — standing rule 1 with the units filed off, and invisible to
  the linter that enforces it.
- **Unmount-after-exit-transition.** The duration tokens collapse to `0ms` under
  `prefers-reduced-motion`, and a 0ms transition never fires `transitionend`. Listening for
  that event would leak the element forever for exactly the users who asked for less
  motion. `Toast` and `Sheet` read the duration and use a timer instead.

`lengthToken` accepts only `px`: a token in `em` or `%` has no fixed pixel value outside
the element it is used on, and treating `0.5em` as 0.5 would place a menu half a pixel from
its trigger. Each call carries a fallback constant, reached only when the cascade has not
been applied — jsdom, essentially — and documented as mirroring the authored token.

---

## 4. Deviations and open questions

### 4.1 `--dur-sheet` is 320ms; standing rule 7 says 100–250ms — **needs your decision**

`docs/02` §4 defines `--dur-sheet: 320ms` and assigns it to "popover / compose open".
`PROMPT.md` standing rule 7 says *every animation is 100–250ms*, with overshoot permitted
only for window-open and the send whoosh — it says nothing about exceeding 250ms.

`PROMPT.md` wins by its own terms, so **Popover, Sheet and Toast animate on `--dur-base`
(200ms)**, and the 320ms token is left defined but unused until Phase 7 decides what the
compose window does. Both readings are defensible and this is a visible difference, so it
is yours to settle. The token is not deleted either way.

### 4.2 Tokens added beyond the doc's literal list

The phase prompt says "every token in the doc, nothing extra, nothing missing". These are
extra. Each one exists because a §6 component specification requires the value and §3/§4
does not name it:

| Token | Why |
|---|---|
| `--focus-ring` | §6.5 gives the ring once, inline, then says "focus ring everywhere" — which makes it a role, not a per-component value. Naming it means it follows the accent, including the inactive-window desaturation, for free. |
| `--destructive`, `--destructive-hover`, `--destructive-fg` | §6.5 gives the destructive button `--flag-red` and no hover state. A filled button needs one; deriving it exactly as `--accent-hover` is derived keeps the two buttons behaving identically. |
| `--scrim` | §6 implies a modal sheet; nothing defines what sits behind one. Greyscale, per standing rule 2. |
| `--scrollbar-thumb`, `--scrollbar-thumb-hover`, and the `--scrollbar-*` metrics | `docs/01` §9.8 asks for momentum scrolling rubber-banded at the ends and never janky. Windows' stock scrollbar is neither macOS-like nor subtle. |
| `--filter-sidebar`, `--filter-header`, `--filter-menu` | §5's three materials as roles, so Reduce Transparency can switch the blur off from one place. Setting them to `none` rather than `blur(0)` is what actually buys back the GPU cost — any radius forces a compositing layer and a readback per frame. |
| `--blur-*`, `--saturate-material` | The raw radii §5 hardcodes inside its `.material-*` classes, moved to the primitive tier where raw values belong. |
| `--press-scale`, `--plain-hover-opacity`, `--focus-ring-*`, `--tint-*` | Scale and opacity constants named in §6 prose. They are sizes too, and standing rule 1 covers them. |
| `--dur-shimmer`, `--ease-linear` | §6.10 specifies "shimmer 1.2s linear" and nothing in §4 can express it. |
| `--menu-submenu-delay`, `--tooltip-delay`, `--tooltip-group-timeout`, `--toast-dwell` | §6.9 specifies the 150ms submenu delay; the others are the same class of value for primitives §6 does not spec at all. |
| `--list-row-height-0/-1/-2` per density | §6.3 gives 46/62/78 at default density; §7 pins only the two-line figure for compact and comfortable. The other two step by the same amount within each mode. **Unverified against `assets/reference/` — Phase 2's job.** |

### 4.3 Inter `opsz`, not the plain weight axis

`docs/01` §10 is reproducing SF Pro's split between Text (< 20pt) and Display (≥ 20pt).
Inter's optical-size axis does the same job continuously — tighter spacing and thinner
hairlines as the size grows — so the `opsz` cut is the closest available analogue. The
family name is `Inter Variable`, which is what Fontsource publishes; the font stack in
`primitive.css` leads with it and keeps `Inter` and the Segoe fallbacks behind.

`font-display: swap` comes from the Fontsource stylesheet. The files are local so the swap
window is effectively never entered in the packaged app, but a cold first paint could in
principle show one frame of Segoe. Not observed. Worth re-checking in Phase 11 on a cold
machine.

### 4.4 The close button on a chip is laid out, not inserted

`docs/02` §6.6 says the x button "appears on hover". Making it appear by entering the
layout would resize the chip and shove every chip after it sideways, which standing rule 6
forbids outright. It is always laid out and fades in. The same reasoning applies to
`TextField`'s clear button and to `MenuItem`'s leading icon column.

### 4.5 The scrollbar gutter is reserved permanently

Chromium lays out a styled `::-webkit-scrollbar` as a classic scrollbar, not an overlay
one. The alternative is content reflowing by 15px the moment a list grows long enough to
scroll, which standing rule 6 forbids — so the gutter is always reserved and the thumb
fades in while scrolling or hovering. Phase 2 checks the resulting text inset against
`assets/reference/`.

`scrollbar-width` and `scrollbar-color` are deliberately absent everywhere. Chromium drops
every `::-webkit-scrollbar` rule on an element that specifies the standard properties, so
one well-meant line "for Firefox" would silently revert this to a stock Windows scrollbar.

### 4.6 Floating layers in the gallery take the page theme

Menus, popovers, tooltips, sheets and toasts render through a portal on `document.body`,
so in the two-column gallery they wear the *page* theme rather than their column's. The
header's theme control is what shows those in both. Fixing it would mean giving every
floating primitive a portal-root prop that only the gallery would ever pass, and standing
rule 18 is against shipping scaffolding for a dev tool's benefit.

### 4.7 `unbound-method` disabled for five files

`@floating-ui/react` declares `refs.setReference` and `refs.setFloating` as methods, so
`@typescript-eslint/unbound-method` fires every time one is passed to a `ref` prop — which
is the library's documented and only usage. Wrapping them in arrow functions to satisfy the
rule would create a new ref callback on every render, detaching and reattaching the node
each time. The rule is off for `src/ui/{Popover,Tooltip,Menu,ContextMenu,Sheet}.tsx` and
nowhere else.

---

## 5. Not verified

- **Display scaling at 125 / 150 / 175 %.** The definition of done requires all four. The
  gallery runs in Chromium at 1 dpr; the Tauri window was launched at the machine's 150 %
  but not compared against 100 % systematically. Needs a manual pass, or a
  `deviceScaleFactor` matrix in the Playwright project list. Carried into Phase 2.
- **`assets/reference/` comparison.** Still empty, so no primitive's metrics have been
  checked against real macOS Mail. Every measurement here comes from `docs/02`. This blocks
  the Phase 2 exit gate and needs half an hour on a Mac running Sequoia.
- **Screen-reader pass.** Roles, names and live regions are asserted in tests, but Narrator
  has not been run against the gallery. Scheduled for Phase 10.
- **Contrast audit.** `--label-2` on `--bg-content` ≥ 4.5:1 (docs/02 §8) has not been
  measured for the new surfaces.

---

## 6. Incidents

- **The first committed visual baseline was a blank white page.** `page.goto` resolves on
  `load`, but `main.tsx` reaches `/dev/gallery` through a dynamic import, so at that moment
  the document was still empty — and `document.fonts.ready` resolved immediately because no
  font had been requested yet. The screenshot test was the *fastest* of the five, which is
  what gave it away. The spec now waits for both theme columns to be visible before
  measuring or capturing. Caught only by opening the PNG; nothing in the test output was
  wrong.

- **The baseline then covered a third of the primitives.** `fullPage: true` captures the
  document, and this document never scrolls — the gallery scrolls inside a `ScrollArea`. So
  the shot was one viewport of specimens and no evidence at all about the twelve primitives
  below the fold. The spec now measures the scroller's content height and grows the viewport
  to it before capturing.

- **Smart App Control blocked the Rust build again, and the recorded guidance is wrong.**
  `cargo clippy` with a fresh `CARGO_TARGET_DIR` at `%LOCALAPPDATA%\mailbox-target` failed
  with `os error 4551` on `icu_normalizer_data`'s build script, and failed identically on
  retry. The same command against the project-local `src-tauri/target` — which is *inside*
  `Documents`, the location `PHASE-0-VERIFICATION.md` §3 and `CLAUDE.md` both say to avoid
  — succeeded immediately.

  The determining factor is therefore not the directory. It is whether a build-script
  executable has to be freshly compiled and then run: `src-tauri/target` already holds
  those executables and their cached output from Phase 0, so nothing new is executed and
  nothing is blocked. Phase 0 changed two variables at once (location and elapsed time) and
  attributed the fix to the wrong one.

  **Revised guidance: keep using the existing `src-tauri/target`.** A fresh target
  directory anywhere will hit this on first build; retrying does not clear it. If a clean
  build is ever needed, expect to fight Smart App Control for the first compile of each new
  dependency set.

- **One Vitest case was order-dependent.** The submenu keyboard test passed alone and in a
  single-file run, and failed inside `npm run verify`. Focus moves in a React effect rather
  than in the keydown handler, and `userEvent` does not flush effects between keystrokes, so
  a synchronous `toHaveFocus()` assertion passed or failed on scheduler interleaving. Every
  focus assertion in the menu suite now awaits settlement; the suite was run three times
  consecutively to confirm.

- **Floating UI's `aria-labelledby` was silently overriding our `aria-label`.** `useRole`
  labels a menu by its trigger, and `aria-labelledby` outranks `aria-label` — which left a
  context menu (no trigger text) anonymous and a submenu named after the row that opened it
  rather than after itself. The floating primitives now spread the interaction props first
  and set their ARIA afterwards, clearing `aria-labelledby` where they supply their own
  name. Caught by a test asserting the accessible name, not by inspection.
