/**
 * The document a message body is shown in. docs/03 §6.
 *
 * A pure function in a file of its own, apart from `MessageFrame`, so it can be run in a real
 * browser by `tests/e2e/messageStylesheet.spec.ts`. Nothing else can test it: jsdom has no
 * layout, and the browser build has no message bodies to show — it only ever renders a
 * placeholder, because there is no core in a browser to sanitise one.
 */

/** docs/03 §6.5, verbatim. */
export const CSP = "default-src 'none'; img-src cid: app: data:; style-src 'unsafe-inline';"

/**
 * Styling for the frame's document, not the app's.
 *
 * It cannot use the token layer: the frame is a separate document and CSS custom properties
 * do not cross that boundary. The colours are passed in from the resolved theme instead, so
 * a message still reads correctly in dark mode — mail is overwhelmingly written for a white
 * background, so the default is a light card even in dark mode, which is what Mail does.
 *
 * ## The message's own stylesheet, and the three layers around it
 *
 * `css` is the sender's `<style>` content, filtered by the core (`mail/css.rs`) so that it can
 * load nothing and contains no `<`. It arrives apart from the body HTML and goes here, in the
 * head, because a stylesheet has to be able to style `<body>` — the Pi-hole report that found
 * this puts its grey page and its padding there — and that is also what lets a message undo
 * the rules this frame cannot do without. So the cascade is split in three, declared first so
 * nothing later can reorder them:
 *
 * - **`halcyon-guard`** — what the frame's measuring and scrolling depend on. Every rule is
 *   `!important`, and among `!important` declarations the *first* layer wins, so nothing a
 *   message writes can override these: not an important rule, not an unlayered one, not one in
 *   a layer of its own. The filter refuses the name `halcyon` in a message's CSS outright,
 *   because a layer is joined by naming it — and a message rule inside this layer would come
 *   after these and win.
 * - **`halcyon-base`** — the frame's defaults, which a message's stylesheet is meant to beat.
 * - **`message`** — the sender's.
 *
 * Inline `style` attributes are not in any layer and never were: element-attached styles beat
 * every rule, which is the behaviour mail has always relied on.
 */
export function frameDocument(
  html: string,
  plainText: boolean,
  css: string,
  hideBlockedImages = false,
): string {
  // Already free of `<` from the core. Replaced again at the boundary regardless, because this
  // is the one place a stray `</style` would matter, and the check costs nothing.
  const sheet =
    css === '' ? '' : `<style>@layer message {\n${css.replaceAll('<', '\\3c ')}\n}</style>`

  return `<!doctype html>
<html><head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${CSP}">
<base target="_blank">
<style>
@layer halcyon-guard, halcyon-base, message;

@layer halcyon-guard {
  /* The frame is sized by measuring its document, so the document must be sized by its content.
     \`html, body { height: 100% }\` is boilerplate in a large share of email templates, and a root
     that fills the frame, plus any padding, is taller than the frame — which is measured, grown,
     filled again and measured again, every frame, without end. */
  html, body { height: auto !important; min-height: 0 !important; max-height: none !important; }

  /* The frame must never scroll itself, and saying so *inside* the document is the only place
     it counts: \`overflow\` on the <iframe> element does not reach the document it contains.

     Without this the reader took two goes to start scrolling. The frame is sized from the
     outside by measuring, so until that lands it is only \`min-height\` tall and its content
     overflows — which makes the inner document a scroll container. Chromium latches a wheel
     gesture to the first scroller it finds under the pointer and keeps it there for the rest of
     the gesture, so the first scroll went nowhere and only a second, separate gesture reached
     the pane behind it.

     \`scrollHeight\` still reports the full content height when overflow is hidden, so the
     measurement above is unaffected. Horizontal scrolling for wide tables is unaffected too:
     that lives on \`.halcyon-scroll\`, an element inside the body. */
  html { overflow: hidden !important; }

  /* The frame is a white card whatever the app's theme. Pinned, so a message's
     \`:root { color-scheme: dark }\` cannot turn system colours like Canvas and CanvasText dark
     inside it. Its \`prefers-color-scheme\` blocks are a different matter: the frame answers those
     from the browser's preference, which nothing here can change, so the core removes them. */
  :root { color-scheme: light !important; }
}

@layer halcyon-base {
  html, body { margin: 0; padding: 0; background: #fff; color: #1c1c1e; }
  body {
    font: 14px/1.55 -apple-system, "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
    padding: 4px 2px 16px;
    word-break: break-word;
    overflow-wrap: anywhere;
  }
  img { max-width: 100%; height: auto; }

  /* A withheld image, in a place that is not allowed to ask for it.
     \`blocked:remote\` is not a real scheme, so the browser draws its broken-image glyph — which
     in the reader is right, because the banner above says what is missing and offers to load
     it. The selection stack has no banner and no offer: it refuses remote images on purpose,
     and a page of broken glyphs there reads as the app failing to load them rather than
     declining to. \`visibility\`, not \`display\`, so the space the sender laid out stays. */
  ${hideBlockedImages ? 'img[src="blocked:remote"] { visibility: hidden; }' : ''}
  /* docs/03 §6.7 — a wide table scrolls inside its own box rather than forcing the page
     sideways. Marketing mail is full of 800px fixed-width tables. */
  table { max-width: 100%; }
  .halcyon-scroll { overflow-x: auto; }
  a { color: #0a58ca; }
  blockquote {
    margin: 0 0 0 8px; padding-left: 12px;
    border-left: 2px solid #d0d0d5; color: #3c3c43;
  }
  pre.halcyon-plain {
    margin: 0; font: inherit; white-space: pre-wrap; word-break: break-word;
  }
  /* The quoted reply, folded. The core wraps it in a <details> because that is the one
     interactive control HTML has that needs no script — and this frame runs none. */
  details.halcyon-quote { margin: 8px 0 0; }
  summary.halcyon-quote-toggle {
    cursor: pointer; display: inline-block; list-style: none;
    padding: 2px 8px; margin: 4px 0;
    border: 1px solid #d0d0d5; border-radius: 10px;
    background: #f5f5f7; color: #3c3c43;
    font-size: 12px; line-height: 1.5; user-select: none;
  }
  summary.halcyon-quote-toggle::-webkit-details-marker { display: none; }
  summary.halcyon-quote-toggle:hover { background: #ebebef; }
  details.halcyon-quote[open] summary.halcyon-quote-toggle { margin-bottom: 8px; }
  .halcyon-quote-body { color: #3c3c43; }
  /* A data detector. Marked with a dotted underline rather than a link colour, because the
     sender did not put a link here and it must not look as though they did. */
  a.halcyon-detected {
    color: inherit; text-decoration: none;
    border-bottom: 1px dashed #9a9aa0; cursor: pointer;
  }
  a.halcyon-detected:hover { border-bottom-color: #0a58ca; color: #0a58ca; }
  ${plainText ? '' : 'body > :first-child { margin-top: 0; }'}
}
</style>
${sheet}
</head><body>${html}</body></html>`
}
