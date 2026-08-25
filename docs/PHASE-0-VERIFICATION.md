# Phase 0 — verification record

Roadmap gate (`docs/04-roadmap.md`): *app launches, window drags/resizes/snaps correctly,
theme follows Windows instantly, caption buttons behave exactly like native ones (including
Snap Layouts hover), CI green on a clean clone.*

**Status: substantially passed, with one item needing a human at the machine and one that
cannot be checked yet.** The app builds, launches and behaves; Snap Layouts was resolved by
giving the caption strip back to Windows (§2). CI has never run, because the repository has
no remote and no commit yet.

---

## 1. Verified

Windows 11 26200, Node 24.19.0, Rust 1.98.0 (`x86_64-pc-windows-msvc`), MSVC 14.44.35207,
Windows SDK 10.0.26100, WebView2 151.0.4129.101.

| Check | Command | Result |
|---|---|---|
| Formatting | `npx prettier --check .` | pass |
| Lint | `npx eslint . --max-warnings 0` | 0 problems |
| Design-token rule | `npx stylelint "src/**/*.css"` | 0 problems |
| Types | `npx tsc --noEmit` | pass |
| Unit tests (UI) | `npx vitest run` | 13 / 13 |
| Production build | `npx vite build` | 215 kB JS, 7.9 kB CSS |
| Shell end-to-end | `npx playwright test` | 6 / 6 |
| Rust formatting | `cargo fmt -- --check` | pass |
| Rust lint | `cargo clippy --all-targets -- -D warnings` | 0 warnings |
| Rust tests | `cargo test` | 4 / 4 |
| Rust build | `cargo build` | `halyard.exe`, 15.2 MB |

The stylelint run enforces standing rule 1: a hex colour, `rgb()` literal, `px` length,
duration or raw easing curve inside any `*.module.css` fails the build.

### Observed in the running app

Launched and captured. From the core's own log and the on-screen diagnostics panel:

- **Window launches** with no white flash, correct theme from the first frame.
- **`system backdrop applied effective=MicaAlt`** — the DWM material took effect. This is
  the Mica-over-Acrylic decision in §4 working as intended.
- **Theme `dark`**, read from Windows.
- **OS accent `#F7630C`** (the user's Windows accent), read live via `UISettings`, with the
  foreground rule correctly selecting white for it.
- **Display scaling 150 %** detected.
- **The system caption renders** with the app icon, title and the three Windows caption
  buttons, and Mica shows through both it and the toolbar below, so they read as one
  continuous band. See §2.

---

## 2. Resolved: Snap Layouts, by giving the caption back to Windows

`docs/03-architecture.md` §8 says Snap Layouts "works if the custom titlebar leaves a real
maximize button hit-region". It does not, and the reason is structural.

An undecorated Tauri window is hosted like this:

```text
Tauri Window                     <- a subclass here only ever sees the border
├─ TAURI_DRAG_RESIZE_BORDERS     <- covers the entire client area
├─ WRY_WEBVIEW
├─ Chrome_WidgetWin_0 / _1       <- owned by msedgewebview2.exe
└─ Chrome_RenderWidgetHostHWND   <- owned by msedgewebview2.exe
```

Every child spans the full client rect, so Windows routes `WM_NCHITTEST` to the deepest
child under the pointer. Instrumenting every hit test on the top-level window made this
unambiguous: 27 hit tests arrived for the resize borders (`HTTOP`, `HTRIGHT`, all with
client coordinates at or outside the edges) and **zero** for any point inside the client
area — including the exact centre of the maximise button at client (1331, 15.7). The usual
escape, subclassing the children to return `HTTRANSPARENT` so the test falls through,
cannot work either: the `Chrome_*` windows belong to the WebView2 process.

So the window is decorated and Windows keeps the caption strip. It draws and hit-tests the
three buttons itself, which makes Snap Layouts, hover, press, `Alt`+`Space`,
double-click-to-maximise and screen-reader support native and unable to regress.
`docs/02` §6.1 already specified those buttons as native metrics with Segoe Fluent glyphs,
so drawing them ourselves was only ever an imitation of what Windows draws anyway.

### The cost, stated plainly

macOS Mail has one unified 52px titlebar-and-toolbar. This app now has a ~32px system
caption **above** a 52px toolbar: 84px of chrome against Mail's 52px. That is a real
fidelity loss and it is the single largest visual deviation in the project so far.

It is softened by the backdrop: the toolbar paints no background of its own, so Mica shows
through the caption strip and the toolbar alike and the two read as one continuous
translucent band rather than two stacked bars. Verified in the running window.

Phase 2 should revisit the total height when the toolbar has real controls in it — a
shorter toolbar under the system caption may land closer to Mail's proportions than 52px
does. Do not change it before there is something in the bar to judge.

### What was removed

The custom caption buttons, their tokens (`--caption-*`, `--font-caption-glyph`,
`--win-close-*`), the `WM_NCHITTEST` subclass in `platform/titlebar.rs`, the
`set_caption_button_rects` command, the caption hover/press event channel, and the window
minimise/maximise/close capabilities. None of it has a purpose once Windows owns the
strip, and standing rule 18 says not to carry it.

## 3. Resolved: the Smart App Control build failure

Earlier in this phase, `cargo build` failed with

```
error: failed to run custom build command for `icu_properties_data v2.3.0`
Caused by: An Application Control policy has blocked this file. (os error 4551)
```

confirmed as CodeIntegrity event 3077 against the Smart App Control policy
`{0283ac0f-fff1-49ae-ada1-8a933130cad6}`. Smart App Control is enabled and enforcing on
this machine (`VerifiedAndReputablePolicyState = 1`) and blocks unsigned, locally compiled
executables — which is exactly what a cargo build script is.

**It no longer reproduces.** The build now completes with the target directory outside
`C:\Users\<user>\Documents`, and `cargo-fmt.exe` — blocked earlier from a path with no
spaces and outside Documents — now runs fine untouched. Two things changed between the
failing and succeeding runs (the build output location, and roughly forty minutes of
elapsed time during which a multi-gigabyte Visual Studio install finished), so the cause is
not cleanly attributable. The evidence points at Smart App Control's reputation lookups
being time- and load-sensitive rather than at a fixed path rule.

**Practical guidance:**

- Keep `CARGO_TARGET_DIR` outside `Documents`. This is the configuration that was verified
  working end to end, it costs nothing, and `Documents` is the kind of user-data location
  application-control policies are most suspicious of.
- If a build script is ever blocked again, retry before concluding anything. Cargo caches
  build-script *output*, so once a build gets through, ordinary edit-compile cycles never
  re-run those scripts — the hurdle is one-time per dependency set.
- Turning Smart App Control off is **not** required, and is not recommended: it cannot be
  re-enabled without resetting or reinstalling Windows.

---

## 4. Deviations from the specs, applied deliberately

Each is argued in a comment at the point of use.

| Spec says | Built as | Why |
|---|---|---|
| `docs/01` §1: one unified 52px titlebar+toolbar | **System caption (~32px) above a 52px toolbar** | The only way to get Snap Layouts out of a WebView-hosted window. The largest visual deviation in the project; reasoning and mitigation in §2. |
| `PROMPT.md` step 3, `docs/03` §8: Acrylic backdrop | **Mica Alt** (`DWMSBT_TABBEDWINDOW`) | Acrylic blurs what is behind the window; Mica samples the wallpaper and desaturates when inactive, which is what `docs/01` §3 describes. Verified in the running app. |
| `docs/02` §5: sidebar material is `backdrop-filter: blur(30px)` | DWM material, with opaque fallback | Inside a WebView, `backdrop-filter` blurs page content behind the element — never the desktop. It stays correct for the header and menu materials, which do blur app content. |
| `docs/02` §3: `--accent-hover: #0A6FD8 / #3D9BFF` | Derived via `color-mix` from `--accent` | Fixed hexes are only right while the accent is Apple blue, but the accent follows the OS — this machine's is `#F7630C`. The ratios reproduce the doc's values to within ~1/255. |
| `docs/02` §3: `--accent-fg: #FFFFFF` | Chosen at runtime by accent luminance | White on a yellow OS accent is ~1.6:1. Threshold 0.5 keeps white everywhere macOS uses it and flips only genuinely light accents. |
| `docs/02` §2: type scale as fixed sizes | Offsets from `--font-size-base` | `docs/02` §1 requires density to swap only `component.css`, but density changes the base size and the scale sits outside all three tiers. At the default base of 13px the doc's table is reproduced exactly. |
| `docs/02` §6.1: `-webkit-app-region: drag` | `data-tauri-drag-region` | The CSS property is a Chromium app-shell feature WebView2 does not honour; the attribute is what Tauri v2 implements. |
| `docs/04` Phase 0: "token files in place, empty" | Colour, space, radius, motion and type tiers filled | Standing rule 1 forbids hardcoding, so the chrome built in this phase needs its tokens to exist. `component.css` holds only titlebar and caption values; §6.2–§6.10 remain Phase 1's work. |

---

## 5. Manual checklist

Run `npm run app:dev` with `CARGO_TARGET_DIR` set outside `Documents`.

Verified:

- [x] Window launches, no white flash, correct theme on the first frame.
- [x] Sidebar and toolbar show the DWM material; the core reports `backdrop: micaAlt`.
- [x] Theme read from Windows.
- [x] OS accent read from Windows and applied (`#F7630C` on this machine).
- [x] Display scaling detected (150 % on this machine).
- [x] System caption renders with the app icon, title and native caption buttons.

Needs a human at the machine. These are all now **native Windows behaviour** rather than
anything this app implements, so they are confirmations rather than risks — but they have
not been seen, and scripted mouse positioning proved unreliable on a 150 %-scaled display:

- [ ] **Snap Layouts flyout on maximise hover.** Windows owns the button; this should
      simply work. It is the reason for the §2 change and has not been observed.
- [ ] Caption hover / press / close-red states.
- [ ] Maximise glyph switches to restore when maximised.
- [ ] Dragging the system caption moves the window; double-click maximises.
- [ ] Dragging the app's own toolbar also moves the window (`data-tauri-drag-region`).
- [ ] All eight resize edges and corners.
- [ ] `Alt`+`Space` opens the system menu.

Still to check, and genuinely ours:

- [ ] Switching Windows light/dark recolours the app instantly with no reload.
- [ ] Changing the Windows accent updates the app within ~150 ms.
- [ ] Turning off Windows "Transparency effects" makes the surfaces opaque and the panel
      reports `reduce transparency: on`.
- [ ] Clicking another window desaturates this one (`[data-window-inactive]`).
- [ ] Layout holds at 100 / 125 / 175 % scaling.
- [ ] Narrator reads the window and the diagnostics panel sensibly.

---
## 6. Known gaps

- `assets/reference/` is empty, so nothing can be compared against macOS yet. Blocks the
  Phase 2 gate, not this one; capturing them needs access to a Mac.
- CI has never executed — the repository has no remote and no commit. The workflow runs the
  same commands verified above.
- `src-tauri/capabilities/default.json` was missing until late in the phase. In Tauri v2 a
  webview gets no core permissions without it, so `listen()` and `startDragging()` were
  being denied silently. It now grants a deliberately granular set rather than
  `core:default`; that file is worth reviewing on every phase that adds IPC.
- The Rust unit tests that covered caption hit-testing were deleted with the code they
  tested. They are replaced by tests pinning the `Appearance` IPC contract — the field
  names and `micaAlt`/`none` spellings that `src/lib/appearance.ts` parses — which is now
  the only Rust logic that is not a direct Win32 call.
