# macOS Mail — Full Teardown

> Purpose: capture _exactly_ what makes Apple Mail feel the way it does, at a level of detail
> that a developer (or a coding agent) can rebuild from without ever opening a Mac.

---

## 0. Version baseline

| Era                  | OS                                  | What it looks like                                                                                                                                                                      |
| -------------------- | ----------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Classic vibrancy** | macOS 11 Big Sur → macOS 15 Sequoia | Translucent sidebar, unified toolbar merged into title bar, inset rounded selection pills, hairline separators. **This is the look most people mean when they say "Mail looks great."** |
| **Liquid Glass**     | macOS 26 Tahoe                      | Same skeleton, but toolbar controls float in glass capsules, heavier specular highlights, more rounded window corners, sidebar reads as a floating glass slab.                          |

**Recommendation: build the Sequoia look as the default theme.** It is calmer, far easier to
reproduce faithfully in a WebView, and ages better. Add a `theme: glass` variant later as a
skin on top of the same tokens — do not architect around it.

Everything below is written for the Sequoia baseline, with Tahoe deltas called out as `[Tahoe]`.

---

## 1. Window anatomy

```
┌──────────────────────────────────────────────────────────────────────────────────────┐
│ ●●●  [◧] [del] [arch] [flag] [reply] [replyall] [fwd]        [  Search           ]   │  <- unified titlebar+toolbar (52pt)
├─────────────────┬──────────────────────────┬─────────────────────────────────────────┤
│                 │  Primary Transactions .. │                                          │  <- [15.4+] category tabs
│  v Favourites   │ ┌──────────────────────┐ │   Subject line, big and bold             │
│    Inbox     12 │ │* Sender name    9:41 │ │                                          │
│    Flagged      │ │  Subject line        │ │   (o)  Sender Name <a@b.com>    9:41 AM  │
│    Sent         │ │  Preview text that.. │ │        To: Me                            │
│                 │ └──────────────────────┘ │  ──────────────────────────────────────  │
│  v iCloud       │ ┌──────────────────────┐ │                                          │
│    Inbox        │ │  Sender name    Yest.│ │   Message body renders here, generous    │
│    Drafts       │ │  Subject             │ │   left/right margins, comfortable line   │
│    Trash        │ │  Preview..           │ │   height, images inline.                 │
│                 │ └──────────────────────┘ │                                          │
│  v Smart Mailb. │                          │   ┌────────────┐                         │
│    Unread       │                          │   │ [] file.pdf│  attachment chip        │
│                 │                          │   └────────────┘                         │
├─────────────────┴──────────────────────────┴─────────────────────────────────────────┤
│  Updating... / 1,204 messages, 12 unread                                  (status)    │
└──────────────────────────────────────────────────────────────────────────────────────┘
   <- 200-260pt ->  <- 300-420pt (resizable) ->  <- remainder, min ~420pt ->
```

Three resizable columns. Divider drag persists per-window. Below a total window width of
~1000pt Mail collapses to two columns (list + reader), and below ~700pt to one column with
push-navigation. **Both breakpoints must be reproduced** — this is a big part of why Mail
feels solid when you resize it, and where every Windows client falls apart.

### Alternate layout

View ▸ **Use Classic Layout** flips to: full-width message list on top, reader pane below,
sidebar unchanged. Ship this as a toggle; it is cheap once the panes are components.

---

## 2. Measurements (logical points ≈ CSS px at 100% scaling)

Derived from @2x screenshots. Treat as **targets to verify** against reference captures
dropped in `assets/reference/`, not gospel.

| Element                                    | Value                                                         |
| ------------------------------------------ | ------------------------------------------------------------- |
| Titlebar + toolbar height                  | 52                                                            |
| Toolbar icon button hit area               | 28 x 28, icon 17, stroke ~1.5                                 |
| Toolbar button gap                         | 2 within a group, 20 between groups                           |
| Sidebar default / min / max width          | 232 / 150 / 400                                               |
| Sidebar row height                         | 28 (24 nested), 32 in Large density                           |
| Sidebar row left inset                     | 10 + 18 per nesting level                                     |
| Sidebar selection pill                     | full row minus 8 each side, radius 6                          |
| Sidebar section header                     | height 28, text 11 semibold, tertiary colour                  |
| Sidebar icon size                          | 16, accent or system colour — never grey-on-grey              |
| Sidebar unread badge                       | 11 tabular, secondary colour, 12 from right edge              |
| Message list default / min width           | 360 / 260                                                     |
| List row height, 0 preview lines           | 46                                                            |
| List row height, 1 preview line            | 62                                                            |
| List row height, 2 preview lines (default) | 78                                                            |
| List row padding                           | 16 left, 12 right                                             |
| List row selection                         | inset 8 each side, radius 6                                   |
| Unread dot                                 | 8 diameter, centred in a 22 gutter at x≈10                    |
| Contact photo                              | 30 circle                                                     |
| Reader body side padding                   | 20                                                            |
| Reader header avatar                       | 32 circle                                                     |
| Reader subject                             | 17 semibold (`[Tahoe]` 19)                                    |
| Hairline                                   | 1px, `rgba(0,0,0,0.10)` light / `rgba(255,255,255,0.12)` dark |
| Compose window default                     | 700 x 560                                                     |
| Compose field row height                   | 30                                                            |
| Window corner radius                       | 10 (`[Tahoe]` 14)                                             |
| Control corner radius                      | 6                                                             |
| Popover / menu radius                      | 8 + 1px inner light stroke                                    |

---

## 3. Sidebar

**Structure, top to bottom:**

1. **Favourites** — user-curated. Default: Inbox, VIPs, Flagged, Drafts, Sent. Reorderable by
   drag. Expanding "Inbox" reveals one child row per account.
2. **Smart Mailboxes** — saved searches, gear icon, user-created.
3. **One section per account** (iCloud, Gmail, Exchange…) with its full folder tree.
4. **On My Mac** — local-only mailboxes.

**Behaviours that matter:**

- Disclosure triangles animate open/closed over ~200ms with the rows below sliding, not
  popping. State persists across launches.
- **No hover highlight on the sidebar** — only selection. Windows apps love to add a hover
  fill here; Mail does not, and that is why the sidebar reads as calm.
- Unread counts are right-aligned, tabular figures, and _disappear at zero_ — never "0".
- Drag a message onto a mailbox: the row gets a filled accent pill and the count animates.
- Context menu: New Mailbox, Rename, Delete, Export Mailbox, Rebuild, Get Account Info,
  Use This Mailbox As ▸ (Drafts / Sent / Junk / Trash / Archive).
- Background uses the `sidebar` NSVisualEffectView material: it samples the desktop wallpaper
  behind the window. When the window is inactive it desaturates toward grey.
- Collapsing the sidebar (⌃⌘S) animates width to 0 with content sliding, ~250ms.

---

## 4. Message list — the single most important surface

### Row anatomy (2-line preview, default)

```
┌ 8pt inset ──────────────────────────────────────────────────────────────┐
│ *   (photo)  Sender Name                            9:41 AM   [2]      │  line 1: 13/600 + 12 secondary
│              Subject line goes here                    [clip] [flag]   │  line 2: 13/400
│              Preview text, two lines, ellipsised at                    │  line 3-4: 13/400 tertiary
│              the end of the second line...                             │
└─────────────────────────────────────────────────────────────────────────┘
```

- **Unread dot**: filled accent circle in the left gutter. Read messages leave the gutter
  empty — the gutter is _always reserved_, so nothing shifts horizontally when read state
  changes.
- **Sender**: semibold when unread, regular when read. Display name, falling back to address.
  In Sent/Drafts it shows the _recipient_, prefixed "To: ".
- **Date**: right-aligned, secondary. Relative-adaptive format — `9:41 AM` today, `Yesterday`,
  `Monday`, `12/03/25` older. Never shows a year within the current year.
- **Thread count**: small pill with message count, only when > 1.
- **Preview text**: quoted content, signatures and boilerplate stripped; whitespace collapsed.
  Line count user-settable 0–5 (View ▸ Preview).
- **Line-2 right icons**: paperclip (attachment), coloured flag, reply/forward arrow (you
  replied), mute, calendar (invite).
- **Contact photo**: optional. 30pt circle, initials fallback on a colour derived
  deterministically from the address hash.

### States

| State                      | Treatment                                                        |
| -------------------------- | ---------------------------------------------------------------- |
| Read, unselected           | Transparent, secondary sender                                    |
| Unread, unselected         | Transparent, **semibold** sender + subject, accent dot           |
| Hover                      | `rgba(0,0,0,0.04)` / `rgba(255,255,255,0.05)`, radius 6 — subtle |
| Selected, window focused   | Accent fill, **all text white**, dot turns white                 |
| Selected, window unfocused | Neutral grey `rgba(0,0,0,0.08)`, text keeps normal colour        |
| Multi-selected             | Accent fill; contiguous runs merge into one rounded block        |
| Dragging                   | Rows stack into a fanned deck with a count badge                 |

### Grouping and sort

- Sticky section headers when sorting by date: **Today / Yesterday / Previous 7 Days /
  Previous 30 Days / month names / years**. 11pt semibold, tertiary, sticky under a blurred
  backdrop while scrolling.
- Sort menu at the list header right: Date, From, Subject, Size, Unread, Flags, Attachments;
  ascending/descending; "Organise by Conversation" toggle.

### Conversation behaviour

- One row per _thread_, showing the newest message's data plus a count.
- Selecting a thread renders **all messages stacked vertically** in the reader, oldest at top,
  each collapsed to a single header line except the newest, which is expanded.
- Clicking a collapsed header expands it in place with a height animation.
- Quoted/trimmed content inside a message collapses behind a small grey `•••` chevron.

### Swipe gestures

Two-finger swipe on a row reveals actions and is **rubber-banded and continuous** — the action
colour fills progressively, and past ~50% travel it commits on release.

- Right → Mark as Unread / Read (blue)
- Left → Archive (blue) or Delete (red), Junk, Flag, More

On Windows: bind to horizontal wheel / precision-touchpad pan and to touch. Polish, not phase 1.

---

## 5. Reader pane

```
Subject line, 17pt semibold, up to 2 lines
──────────────────────────────────────────────────────────────────────
(o)32  Sender Name                                      9:41 AM   [..]
       To: Me, Other Person  v                       [reply][all][fwd]
──────────────────────────────────────────────────────────────────────
```

- Recipients collapse to "To: Me" with a chevron that expands full To/Cc lists.
- Hovering the header fades in reply / reply-all / forward glyphs on the right (~120ms).
- Names are chips: click → contact card popover (email, phone, Add to VIPs, Block Contact,
  recent messages from this person).
- A **banner strip** appears above the header for: remote content blocked, message is junk,
  encrypted/signed, this is a draft, sent from a different address, "this may be a scam".

**Body:**

- HTML rendered with 20pt side padding, body 13pt, images scaled to fit width.
- Plain-text mail renders in the **system font, not monospace**, with URLs auto-linked.
- **Data detectors**: dates/times get a dotted underline on hover → "Create Event" popover;
  addresses → maps; phone, flight and tracking numbers similarly.
- Link hover shows the target URL in the status area at the bottom.

**Attachments:**

- Chips at the bottom: icon or thumbnail + filename + size, radius 8, light fill.
- Images may render inline instead (per-message toggle).
- Quick Look on Space / double-click; drag a chip to the desktop to save. "Save All" when > 1.

**Reading behaviours:**

- Marks read after a configurable dwell (default immediate).
- ⌘↓ / ⌘↑ move between messages in a thread; ↑/↓ move rows and the reader **cross-fades**
  (~100ms, no slide).
- Scroll position is remembered per message when you navigate away and back.

---

## 6. Compose window

A **separate floating window**, not a pane. 700x560 default, remembers size.

```
┌───────────────────────────────────────────────────────────┐
│ ●●●          [Aa] [clip] [img] [lock]           [ >  Send]│  toolbar, Send = blue capsule
├───────────────────────────────────────────────────────────┤
│ To:      [Person x] [Another x] |                         │  token field, 30pt row
│ Cc/Bcc:  ...                                   From: v    │
│ Subject: ...                                              │
├───────────────────────────────────────────────────────────┤
│ B I U  A v  = *  1.                (format bar, via Aa)   │
├───────────────────────────────────────────────────────────┤
│  Body, rich text, 13pt                                    │
│  --                                                       │
│  Signature                                                │
└───────────────────────────────────────────────────────────┘
```

- **Recipient tokens**: autocomplete from contacts + previous recipients; Enter/comma commits
  a chip; chips drag between To/Cc/Bcc; invalid addresses go red; duplicates silently deduped.
  Backspace _selects_ the previous chip before deleting it.
- **From** picker appears only with more than one account/alias.
- Attachments appear as chips inline at the insertion point.
- **Send** is a filled capsule top-right (⌘⇧D). After sending, the window does a
  scale-and-fly "whoosh" toward the mailbox.
- **Undo Send**: banner at the bottom of the main window for 10s (10/20/30s or off) — the
  message genuinely is not transmitted until the timer elapses.
- **Send Later**: dropdown beside Send — Tonight 9 PM / Tomorrow 8 AM / Custom.
- Closing an unsaved compose offers Save as Draft / Delete / Cancel. Drafts autosave every
  ~30s and on blur.
- **Markup** on image attachments; **Mail Drop** for >20MB.

---

## 7. Search

Search field lives in the toolbar, right-aligned, ~200pt collapsed, expands on focus.

- Typing produces a **token-based** query. Type "john" → dropdown offers
  `People: John Smith`, `Subject contains: john`, `Mailboxes: ...`, `Attachment named john`.
  Selecting one turns it into a **blue capsule token**. Tokens combine with AND.
- Free text also searches everything, including attachment contents.
- Natural language: "mail from Ana last week with attachments" resolves into tokens.
- Results view adds a **scope bar** above the list — `All / Inbox / Drafts...` plus
  `All / Unread / Flagged` — and a **Top Hits** section pinned above chronological results.
- Matched terms highlight yellow in the reader.
- Any search can be saved as a Smart Mailbox from the results view.

---

## 8. Organisation features

| Feature                | Behaviour                                                                                                                                                                                       |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Flags**              | 7 colours (red/orange/yellow/green/blue/purple/grey), renameable. Flagged smart folder with per-colour children.                                                                                |
| **VIPs**               | Star a sender → their mail lands in a VIP mailbox with its own notification rules. Max 100.                                                                                                     |
| **Smart Mailboxes**    | Saved compound predicates (any/all of from, to, subject, date, contains, has attachment, flagged, unread, mailbox, size, is junk...), nestable in folders.                                      |
| **Rules**              | Same predicate builder plus actions: move, copy, set colour, play sound, forward, reply with template, mark read/flagged, run script, stop evaluating. Runs on incoming mail or manually (⌥⌘L). |
| **Mute thread**        | Silences notifications for a conversation; optionally auto-archives.                                                                                                                            |
| **Block sender**       | Marks and optionally bins all future mail from that address.                                                                                                                                    |
| **Remind Me**          | Snooze: the message leaves the inbox and returns to the top at a chosen time.                                                                                                                   |
| **Follow Up**          | Detects an unanswered sent message and re-surfaces it at the top of the inbox.                                                                                                                  |
| **Categories** (15.4+) | Auto-classifies into Primary / Transactions / Updates / Promotions with tabs above the list; Promotions can render as a visual digest of cards.                                                 |
| **Junk**               | Adaptive filter with a training mode ("Mark as Junk" teaches it); junk gets a warning banner.                                                                                                   |
| **Privacy Protection** | Blocks remote content by default and routes loaded content through a relay so senders cannot read your IP or open time.                                                                         |

---

## 9. The secret sauce — why it feels good

These are what Windows mail clients consistently miss. **Treat this list as the acceptance
criteria for "does it feel like Mail".**

1. **Nothing ever jumps.** Reserved gutters, fixed row heights, skeleton placeholders on load.
   Marking read, flagging, mail arriving — none of it reflows what you are looking at.
2. **Instant local response.** Every action writes to the local DB and updates the UI in the
   same frame; network sync is fire-and-forget with reconciliation. Deleting is 0ms.
3. **Restraint in colour.** The only saturated colour on screen is the accent (selection,
   unread dot, links, Send) and flag colours. Everything else is greyscale.
4. **Hairlines, not borders.** 1px at 10% opacity, never a solid grey line.
5. **Typography carries the hierarchy**, not boxes. Weight and opacity, not fills and rules.
6. **Generous space in the reader**, cramped-but-scannable in the list.
7. **Every animation is short and eased-out.** 100–250ms, `cubic-bezier(0.25, 0.1, 0.25, 1)`.
   Nothing bounces except window scale and the send whoosh.
8. **Momentum scrolling, rubber-banded at the ends**, never janky — list is virtualised and
   rows are cheap.
9. **Keyboard-complete.** You can triage a hundred messages without touching the mouse.
10. **Text is always selectable**, everywhere, and copy preserves formatting.
11. **The window goes quiet when inactive** — colours desaturate, selection greys out.
12. **Sound design**: soft whoosh on send, subtle chime on new mail. Off by default is fine.

---

## 10. Typography

macOS uses **SF Pro Text** (< 20pt) and **SF Pro Display** (≥ 20pt). SF Pro is licensed for use
_on Apple platforms only_ — see `06-risks-and-legal.md`. Substitute:

| Role             | macOS          | Windows substitute                                |
| ---------------- | -------------- | ------------------------------------------------- |
| UI + body        | SF Pro Text 13 | **Inter** 13.5px (Inter Display at large sizes)   |
| Fallback chain   | —              | `Segoe UI Variable Text`, `Segoe UI`, `system-ui` |
| Dates and counts | SF tabular     | Inter with `font-variant-numeric: tabular-nums`   |
| Monospace        | SF Mono        | **JetBrains Mono** or `Cascadia Code`             |

Inter needs tuning to read like SF: enable `cv05`, `cv08`, `ss03`; `letter-spacing: -0.01em`
at body size and `-0.02em` at 17px+; `line-height` 1.35 for UI, 1.55 for message bodies.

**Type scale:**

| Token      | Size / Weight | Use                       |
| ---------- | ------------- | ------------------------- |
| `caption`  | 10 / 400      | timestamps in dense mode  |
| `footnote` | 11 / 500      | section headers, badges   |
| `subhead`  | 12 / 400      | dates, secondary metadata |
| `body`     | 13 / 400      | everything                |
| `headline` | 13 / 600      | unread sender, emphasis   |
| `title3`   | 15 / 600      | reader subject, compact   |
| `title2`   | 17 / 600      | reader subject            |
| `title1`   | 22 / 700      | empty states, onboarding  |

---

## 11. Colour

Apple's semantic label colours are **opacities of black/white over the surface**, not fixed
greys. This is _why_ Mail's greys look right in both themes.

| Token                 | Light                   | Dark                     |
| --------------------- | ----------------------- | ------------------------ |
| label (primary)       | `rgba(0,0,0,0.85)`      | `rgba(255,255,255,0.85)` |
| secondaryLabel        | `rgba(0,0,0,0.50)`      | `rgba(255,255,255,0.55)` |
| tertiaryLabel         | `rgba(0,0,0,0.26)`      | `rgba(255,255,255,0.25)` |
| quaternaryLabel       | `rgba(0,0,0,0.10)`      | `rgba(255,255,255,0.10)` |
| separator             | `rgba(0,0,0,0.10)`      | `rgba(255,255,255,0.12)` |
| window background     | `#FFFFFF`               | `#1E1E1E`                |
| sidebar over material | `#F2F2F7` @ ~70% + blur | `#2A2A2C` @ ~70% + blur  |

System accents (sRGB), light / dark:

| Colour                | Light     | Dark      |
| --------------------- | --------- | --------- |
| Blue (default accent) | `#007AFF` | `#0A84FF` |
| Red                   | `#FF3B30` | `#FF453A` |
| Orange                | `#FF9500` | `#FF9F0A` |
| Yellow                | `#FFCC00` | `#FFD60A` |
| Green                 | `#28CD41` | `#32D74B` |
| Mint                  | `#00C7BE` | `#63E6E2` |
| Teal                  | `#59ADC4` | `#6AC4DC` |
| Indigo                | `#5856D6` | `#5E5CE6` |
| Purple                | `#AF52DE` | `#BF5AF2` |
| Pink                  | `#FF2D55` | `#FF375F` |
| Graphite              | `#8E8E93` | `#98989D` |

**On Windows: read the user's accent colour from the OS** and use it as the default accent,
with an in-app override offering the Apple palette. macOS does exactly this, and matching the
OS accent is a large part of feeling native.

---

## 12. Materials and depth

Mail uses four NSVisualEffectView materials. WebView2 equivalents:

| macOS material     | Where                         | Windows recipe                                                                                                                                            |
| ------------------ | ----------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sidebar`          | Sidebar                       | Host window uses **Acrylic** system backdrop; sidebar div is transparent. Fallback: `backdrop-filter: blur(30px) saturate(180%)` over a 70%-opacity base. |
| `headerView`       | Toolbar, sticky list headers  | `backdrop-filter: blur(20px)` + 80% surface + bottom hairline                                                                                             |
| `menu` / `popover` | Menus, contact cards, pickers | `blur(30px)`, 92% surface, radius 8, 1px inner light stroke, `box-shadow: 0 8px 28px rgba(0,0,0,0.18)`                                                    |
| `windowBackground` | Reader, compose               | Opaque                                                                                                                                                    |

Shadows are used **only** on floating layers (menus, popovers, compose window, drag deck).
Never on list rows, cards, or buttons. `[Tahoe]` adds a specular top-edge highlight
(`inset 0 1px 0 rgba(255,255,255,0.25)`) on glass surfaces.

---

## 13. Icons

Mail is entirely **SF Symbols** — one stroke weight, optically consistent, monochrome except
where semantic (flag colours). On Windows:

- Use **Lucide** (1.5px stroke, 24px grid) as the base set, or **Phosphor** Regular.
- Do **not** mix icon sets. Do not use Fluent icons — they read as Office, which is the
  aesthetic you are escaping.
- Never ship SF Symbols glyphs themselves (licensing — see risks doc).
- Toolbar icons: 17px in a 28px hit target, stroke 1.5, colour = secondaryLabel → label on
  hover → accent when active/toggled.

---

## 14. Keyboard model

Ship _all_ of these. Map ⌘ → Ctrl. This table is the phase-1 keyboard spec.

| Action                   | macOS   | Windows                                        |
| ------------------------ | ------- | ---------------------------------------------- |
| New message              | ⌘N      | Ctrl+N                                         |
| Send                     | ⌘⇧D     | Ctrl+Enter                                     |
| Reply                    | ⌘R      | Ctrl+R                                         |
| Reply All                | ⇧⌘R     | Ctrl+Shift+R                                   |
| Forward                  | ⇧⌘F     | Ctrl+Shift+F                                   |
| Redirect                 | ⇧⌘E     | Ctrl+Shift+E                                   |
| Archive                  | ⌃⌘A     | Ctrl+Shift+A                                   |
| Delete                   | ⌘⌫      | Delete                                         |
| Delete permanently       | ⌥⌘⌫     | Shift+Delete                                   |
| Mark read/unread         | ⇧⌘U     | Ctrl+U                                         |
| Flag                     | ⇧⌘L     | Ctrl+L                                         |
| Mark as junk             | ⇧⌘J     | Ctrl+J                                         |
| Move to…                 | ⇧⌘M     | Ctrl+Shift+M                                   |
| Search                   | ⌘⌥F     | Ctrl+F                                         |
| Get new mail             | ⇧⌘N     | F5                                             |
| Next / prev message      | ↓ / ↑   | ↓ / ↑                                          |
| Next / prev in thread    | ⌘↓ / ⌘↑ | Ctrl+↓ / Ctrl+↑                                |
| Toggle sidebar           | ⌃⌘S     | Ctrl+Shift+S                                   |
| Jump to mailbox 1–9      | ⌘1–9    | Ctrl+1–9                                       |
| Expand / collapse thread | → / ←   | → / ←                                          |
| Preview attachment       | Space   | Space                                          |
| Undo                     | ⌘Z      | Ctrl+Z — must undo delete, move, archive, send |

---

## 15. Where Windows clients fail (the gap you are filling)

| Client                     | What it gets wrong                                                                                                                                                          |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Outlook (new)**          | A web app in a shell. Ads in the free tier. Ribbon-derived toolbar with 40+ affordances. Latency on every action. Routes personal IMAP through Microsoft's cloud.           |
| **Outlook (classic)**      | Dense, Win32-era, deeply capable, visually exhausting.                                                                                                                      |
| **Thunderbird**            | Genuinely powerful and local-first, but the default theme is a Firefox-chrome pastiche — grey boxes, borders everywhere, mismatched icon weights, no typographic hierarchy. |
| **Mailbird / eM Client**   | Closest visually, but "skinned" — flat fills, heavy borders, inconsistent spacing, and paywalls at 2–3 accounts.                                                            |
| **Windows Mail (retired)** | Was closest in spirit; discontinued and redirected to new Outlook.                                                                                                          |
| **Spark / Canary**         | Good design, but cloud-mediated — a dealbreaker if you want credentials and mail to stay local.                                                                             |

**Your differentiators, in order:** (1) it looks and feels like Mail, (2) everything is local
and instant, (3) no cloud middleman for credentials or mail, (4) no ads, no account limit.

---

## 16. Replicate / Adapt / Drop

| Replicate exactly                   | Adapt to Windows                                                   | Drop for v1                          |
| ----------------------------------- | ------------------------------------------------------------------ | ------------------------------------ |
| 3-pane layout + responsive collapse | Traffic lights → Windows caption buttons, custom-drawn, right side | Mail Drop (needs iCloud)             |
| List row anatomy and states         | ⌘ → Ctrl, ⌥ → Alt                                                  | Stationery                           |
| Type scale and label opacities      | Toast notifications with actions                                   | AppleScript rules                    |
| Hairlines, radii, spacing           | Windows accent colour as default                                   | Handoff / Continuity                 |
| Motion durations and easing         | Acrylic/Mica instead of NSVisualEffectView                         | iCloud+ Hide My Email                |
| Sidebar structure and behaviour     | Explorer drag-and-drop for attachments                             | Apple Intelligence summaries         |
| Compose window and token fields     | Jump List + taskbar unread badge                                   | Quick Look → built-in previewer      |
| Undo Send, Send Later, Remind Me    | `mailto:` protocol registration                                    | Live Text in images                  |
| Smart Mailboxes, Rules, VIPs, Flags | MSIX/NSIS installer + auto-update                                  | Markup pen input                     |
| Search tokens + Top Hits            | Windows Search integration (optional)                              | Categories (v2 — needs a classifier) |
