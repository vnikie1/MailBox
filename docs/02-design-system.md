# Design System — tokens, primitives, component specs

Everything visual derives from this file. **No component may hardcode a colour, size, radius,
duration or easing value.** If a value is needed and not here, add it here first.

---

## 1. Token layer model

```
tokens/primitive.css   raw values      --blue-500: #007AFF
tokens/semantic.css    roles           --accent: var(--blue-500)
tokens/component.css   per-component   --list-row-height: 78px
```

Themes swap only `semantic.css`. Density swaps only `component.css`.

---

## 2. Font

```css
@font-face {
  /* Inter var, self-hosted, do not CDN */
}

:root {
  --font-ui: 'Inter', 'Inter var', 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif;
  --font-mono: 'JetBrains Mono', 'Cascadia Code', ui-monospace, monospace;

  /* SF-alike tuning */
  --font-feat-ui: 'cv05' 1, 'cv08' 1, 'ss03' 1, 'calt' 1;
  --font-feat-num: 'tnum' 1;
}

body {
  font-family: var(--font-ui);
  font-feature-settings: var(--font-feat-ui);
  -webkit-font-smoothing: antialiased;
  text-rendering: optimizeLegibility;
}
```

### Type scale

| Token          | size / weight / line-height / tracking |
| -------------- | -------------------------------------- |
| `--t-caption`  | 10px / 400 / 1.3 / 0                   |
| `--t-footnote` | 11px / 500 / 1.3 / 0.01em              |
| `--t-subhead`  | 12px / 400 / 1.35 / -0.005em           |
| `--t-body`     | 13px / 400 / 1.35 / -0.01em            |
| `--t-headline` | 13px / 600 / 1.35 / -0.01em            |
| `--t-title3`   | 15px / 600 / 1.3 / -0.015em            |
| `--t-title2`   | 17px / 600 / 1.28 / -0.02em            |
| `--t-title1`   | 22px / 700 / 1.22 / -0.022em           |
| `--t-mailbody` | 13px / 400 / **1.55** / -0.005em       |

---

## 3. Colour tokens — paste-ready

```css
/* ---------- primitive ---------- */
:root {
  --blue-l: #007aff;
  --blue-d: #0a84ff;
  --red-l: #ff3b30;
  --red-d: #ff453a;
  --orange-l: #ff9500;
  --orange-d: #ff9f0a;
  --yellow-l: #ffcc00;
  --yellow-d: #ffd60a;
  --green-l: #28cd41;
  --green-d: #32d74b;
  --mint-l: #00c7be;
  --mint-d: #63e6e2;
  --teal-l: #59adc4;
  --teal-d: #6ac4dc;
  --indigo-l: #5856d6;
  --indigo-d: #5e5ce6;
  --purple-l: #af52de;
  --purple-d: #bf5af2;
  --pink-l: #ff2d55;
  --pink-d: #ff375f;
  --gray-l: #8e8e93;
  --gray-d: #98989d;
}

/* ---------- semantic : light ---------- */
:root,
[data-theme='light'] {
  --accent: var(--blue-l);
  --accent-hover: #0a6fd8;
  --accent-fg: #ffffff;

  --label-1: rgba(0, 0, 0, 0.85);
  --label-2: rgba(0, 0, 0, 0.5);
  --label-3: rgba(0, 0, 0, 0.26);
  --label-4: rgba(0, 0, 0, 0.1);

  --bg-window: #ffffff;
  --bg-content: #ffffff;
  --bg-sidebar: rgba(242, 242, 247, 0.72);
  --bg-header: rgba(255, 255, 255, 0.8);
  --bg-menu: rgba(250, 250, 252, 0.92);
  --bg-raised: #f5f5f7; /* attachment chips, code blocks */

  --fill-hover: rgba(0, 0, 0, 0.04);
  --fill-pressed: rgba(0, 0, 0, 0.08);
  --fill-selected-inactive: rgba(0, 0, 0, 0.08);

  --separator: rgba(0, 0, 0, 0.1);
  --stroke-inner: rgba(255, 255, 255, 0.6);

  --shadow-popover: 0 8px 28px rgba(0, 0, 0, 0.18), 0 0 0 0.5px rgba(0, 0, 0, 0.1);
  --shadow-window: 0 24px 64px rgba(0, 0, 0, 0.28), 0 0 0 0.5px rgba(0, 0, 0, 0.14);

  --flag-red: var(--red-l);
  --flag-orange: var(--orange-l);
  --flag-yellow: var(--yellow-l);
  --flag-green: var(--green-l);
  --flag-blue: var(--blue-l);
  --flag-purple: var(--purple-l);
  --flag-gray: var(--gray-l);
}

/* ---------- semantic : dark ---------- */
[data-theme='dark'] {
  --accent: var(--blue-d);
  --accent-hover: #3d9bff;
  --accent-fg: #ffffff;

  --label-1: rgba(255, 255, 255, 0.85);
  --label-2: rgba(255, 255, 255, 0.55);
  --label-3: rgba(255, 255, 255, 0.25);
  --label-4: rgba(255, 255, 255, 0.1);

  --bg-window: #1e1e1e;
  --bg-content: #1e1e1e;
  --bg-sidebar: rgba(42, 42, 44, 0.72);
  --bg-header: rgba(30, 30, 30, 0.8);
  --bg-menu: rgba(44, 44, 46, 0.92);
  --bg-raised: #2a2a2c;

  --fill-hover: rgba(255, 255, 255, 0.05);
  --fill-pressed: rgba(255, 255, 255, 0.09);
  --fill-selected-inactive: rgba(255, 255, 255, 0.11);

  --separator: rgba(255, 255, 255, 0.12);
  --stroke-inner: rgba(255, 255, 255, 0.1);

  --shadow-popover: 0 8px 28px rgba(0, 0, 0, 0.55), 0 0 0 0.5px rgba(255, 255, 255, 0.1);
  --shadow-window: 0 24px 64px rgba(0, 0, 0, 0.65), 0 0 0 0.5px rgba(255, 255, 255, 0.12);

  --flag-red: var(--red-d);
  --flag-orange: var(--orange-d);
  --flag-yellow: var(--yellow-d);
  --flag-green: var(--green-d);
  --flag-blue: var(--blue-d);
  --flag-purple: var(--purple-d);
  --flag-gray: var(--gray-d);
}
```

**Rule:** the _only_ saturated colour permitted in chrome is `--accent` and the flag colours.
Any other colour in a component spec is a bug.

**Inactive window:** when the window loses focus, set `[data-window-inactive]` on `<html>` and
map `--accent → var(--gray-l/d)` plus reduce `--label-1` to `--label-2`. One rule, whole app.

---

## 4. Space, radius, motion

```css
:root {
  --sp-1: 2px;
  --sp-2: 4px;
  --sp-3: 6px;
  --sp-4: 8px;
  --sp-5: 10px;
  --sp-6: 12px;
  --sp-7: 16px;
  --sp-8: 20px;
  --sp-9: 24px;
  --sp-10: 32px;

  --r-sm: 4px;
  --r-md: 6px;
  --r-lg: 8px;
  --r-xl: 10px;
  --r-2xl: 14px;
  --r-pill: 999px;

  --hairline: 1px;

  /* motion */
  --ease-out: cubic-bezier(0.25, 0.1, 0.25, 1); /* Apple default */
  --ease-in-out: cubic-bezier(0.42, 0, 0.58, 1);
  --ease-spring: linear(
    0,
    0.006,
    0.026,
    0.056,
    0.096,
    0.144,
    0.198,
    0.257,
    0.318,
    0.38,
    0.44,
    0.497,
    0.55,
    0.599,
    0.643,
    0.682,
    0.716,
    0.746,
    0.771,
    0.793,
    0.812,
    0.828,
    0.842,
    0.854,
    0.864,
    0.873,
    0.881,
    0.888,
    0.909,
    0.926,
    0.94,
    0.951,
    0.96,
    0.968,
    0.974,
    0.98,
    0.984,
    0.988,
    0.991,
    0.994,
    0.996,
    0.998,
    1
  );

  --dur-micro: 100ms; /* cross-fade, icon state */
  --dur-fast: 150ms; /* hover, focus ring */
  --dur-base: 200ms; /* disclosure, panel resize */
  --dur-slow: 250ms; /* sidebar collapse */
  --dur-sheet: 320ms; /* popover / compose open */
}

@media (prefers-reduced-motion: reduce) {
  :root {
    --dur-micro: 0ms;
    --dur-fast: 0ms;
    --dur-base: 0ms;
    --dur-slow: 0ms;
    --dur-sheet: 0ms;
  }
}
```

Only `--ease-spring` is allowed to overshoot, and only on compose-window open and send-whoosh.

---

## 5. Materials

```css
.material-sidebar {
  background: var(--bg-sidebar);
  backdrop-filter: blur(30px) saturate(180%);
}
.material-header {
  background: var(--bg-header);
  backdrop-filter: blur(20px) saturate(180%);
  border-bottom: var(--hairline) solid var(--separator);
}
.material-menu {
  background: var(--bg-menu);
  backdrop-filter: blur(30px) saturate(180%);
  border-radius: var(--r-lg);
  box-shadow: var(--shadow-popover);
  border: var(--hairline) solid var(--stroke-inner);
}
```

Enable the host-window Acrylic backdrop and make `<body>` transparent so the sidebar samples
the desktop. Provide a settings toggle **Reduce transparency** that swaps all three to opaque
fills — required for accessibility and for weak GPUs.

---

## 6. Component specs

### 6.1 Titlebar / toolbar

- Height 52. `-webkit-app-region: drag` on the bar, `no-drag` on every control.
- Windows caption buttons drawn in-app on the **right** (46x32 each, Segoe Fluent Icons
  glyphs `   `), hover `rgba(0,0,0,0.05)`, close hover `#C42B1C` + white.
- Left group: sidebar toggle | delete, archive, junk | reply, reply-all, forward | flag, move.
- Right: search field, then caption buttons.
- Buttons: 28x28, radius `--r-md`, icon 17, colour `--label-2`; hover fill `--fill-hover` and
  colour `--label-1`; active/toggled colour `--accent`; disabled colour `--label-3`, no fill.

### 6.2 Sidebar row

```
height 28 | radius 6 | inset 8 each side | icon 16 @ accent | gap 8 | label --t-body
selected: bg --accent, all text/icon --accent-fg
selected + window inactive: bg --fill-selected-inactive, colours unchanged
hover: NONE
badge: --t-footnote, --label-2, tabular, right 12; hidden at 0; --accent-fg when selected
drop target: bg --accent at 0.25 alpha + 1px accent inset ring
```

Section header: height 28, `--t-footnote`, `--label-3`, uppercase off, left inset 10,
disclosure chevron 10px that rotates 90deg over `--dur-base`.

### 6.3 Message list row

```
height: --list-row-height (46 / 62 / 78 by preview lines)
padding: 0 12 0 16 | selection inset 8 each side | radius 6
grid: [22 gutter] [30 photo, optional] [1fr content] [auto meta]

line 1: sender (--t-headline when unread, --t-body --label-1 when read)
        date   (--t-subhead, --label-2, tabular, right)
line 2: subject (--t-body, --label-1)  + icons right (14px, --label-2/flag colour)
line 3+: preview (--t-body, --label-3, -webkit-line-clamp: N)

unread dot: 8px circle --accent at gutter centre
selected(focused): bg --accent; sender/subject/preview/date/icons ALL --accent-fg
selected(blurred): bg --fill-selected-inactive; colours unchanged
hover(unselected): bg --fill-hover
```

Sticky date header: height 26, `--t-footnote`, `--label-3`, `.material-header` backdrop, no
bottom hairline.

### 6.4 Reader header

```
subject: --t-title2, --label-1, clamp 2 lines, padding 20 20 12
rule: hairline full-bleed
row: avatar 32 circle | name --t-headline | address --t-subhead --label-2
     right: date --t-subhead --label-2, then action glyphs (opacity 0 -> 1 on header hover, --dur-fast)
recipients: --t-subhead --label-2, chevron expands full list, --dur-base height animation
banner: height 36, bg = colour at 0.12 alpha, 1px hairline, icon 14 + --t-subhead + inline action button
```

### 6.5 Buttons

| Variant           | Spec                                                                                                                                    |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| **Filled** (Send) | height 28, padding 0 14, radius `--r-pill`, bg `--accent`, text `--t-headline`/`--accent-fg`; hover `--accent-hover`; active scale 0.97 |
| **Bordered**      | height 28, bg `--bg-raised`, 1px `--separator`, radius `--r-md`, text `--label-1`                                                       |
| **Plain**         | text `--accent`, no background, hover underline off / opacity 0.75                                                                      |
| **Icon**          | 28x28, see toolbar spec                                                                                                                 |
| **Destructive**   | as Filled but bg `--flag-red`                                                                                                           |

Focus ring everywhere: `box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 40%, transparent)`,
only on `:focus-visible`.

### 6.6 Search field

```
collapsed 200 wide, focused expands to 320 over --dur-base
height 28, radius --r-md, bg --fill-hover, no border
icon 14 --label-3 at left 8; placeholder --label-3
tokens: inline capsules, height 20, radius --r-pill, bg --accent @0.15, text --accent,
        x button appears on hover; backspace selects then deletes
dropdown: .material-menu, rows 28, grouped with --t-footnote headers, arrow-key navigable
```

### 6.7 Recipient token field (compose)

```
row min-height 30, wraps to multiple lines
chip: height 20, radius --r-pill, bg --fill-hover, text --t-subhead --label-1
      avatar 16 optional; invalid -> bg --flag-red @0.15, text --flag-red
label ("To:") --t-body --label-2, fixed 60 width, right-aligned
```

### 6.8 Attachment chip

```
height 44, radius --r-lg, bg --bg-raised, padding 0 12, gap 8
icon/thumb 28, name --t-body --label-1 truncate, size --t-caption --label-2
hover: --fill-hover overlay; draggable to Explorer
```

### 6.9 Menus and context menus

```
.material-menu | min-width 180 | padding 4
item: height 26, radius --r-sm, padding 0 10, --t-body
      hover: bg --accent, text --accent-fg
      shortcut hint right-aligned --label-3 (--label-... inherits on hover)
separator: 1px --separator, margin 4 8
submenu opens on 150ms hover delay with a safe triangle
```

### 6.10 Empty and loading states

- Empty reader: centred 22px `--label-3` glyph + `--t-title1` `--label-3` "No Message Selected".
- Loading list: 8 skeleton rows, `--fill-hover` bars at 60%/85%/95% width, shimmer 1.2s linear.
- **Never** show a spinner in the list. Sync progress belongs in the status bar only.

---

## 7. Density modes

Ship three, switching only `component.css`:

|                           | Compact | Default | Comfortable |
| ------------------------- | ------- | ------- | ----------- |
| List row (2-line preview) | 66      | 78      | 90          |
| Sidebar row               | 24      | 28      | 32          |
| Toolbar height            | 44      | 52      | 58          |
| Base font                 | 12.5    | 13      | 14          |

---

## 8. Visual QA checklist

Run this before calling any screen done.

- [ ] Screenshot side-by-side with the macOS reference in `assets/reference/` at the same width.
- [ ] Overlay at 50% opacity — row baselines within 2px.
- [ ] No colour on screen outside greyscale + accent + flag colours.
- [ ] No solid grey borders; every divider is a 10%-alpha hairline.
- [ ] Light and dark both pass; toggle at runtime with no reload and no flash.
- [ ] Window inactive state desaturates.
- [ ] All text selectable; tab order sensible; focus ring visible.
- [ ] 100% / 125% / 150% / 175% Windows scaling all render crisply.
- [ ] `prefers-reduced-motion` and Reduce transparency both honoured.
- [ ] Contrast: `--label-2` on `--bg-content` ≥ 4.5:1 for anything the user must read.
