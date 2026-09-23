# Phase 6 — verification record

Written on 2026-09-01, months after the phase was built, because an audit of the exit gates found
this one had almost no recorded evidence behind it. The sanitiser was real and well tested; the
three things the gate actually asks for had not been done.

> Exit gate: the XSS corpus is fully blocked (show the test output); no message causes a network
> request before consent (show a network trace); 20 real newsletters from major senders render
> correctly in both themes.

---

## 1. The XSS corpus — PASS

The gate asks for the corpus to be written first and committed. It was not, so it is now:
`src-tauri/tests/fixtures/xss-corpus.txt`, **69 payloads** across script elements, event handlers,
executable URL schemes, framing and embedding, SVG and MathML, CSS, `meta`/`base`, form controls
and parser-confusion vectors.

`src-tauri/tests/xss_corpus.rs` runs every payload through **`render`** rather than `sanitise`.
That distinction is the point: `sanitise` is the allowlist, and testing it alone tests ammonia's
configuration. What protects a reader is the whole pipeline — sanitise, rewrite images, detect
links, fold the quote — and a hole introduced *after* the allowlist would leave the sanitiser
innocent and the message still dangerous.

    XSS corpus: 69 payloads, images blocked, 0 survived
    XSS corpus: 69 payloads, images allowed, 0 survived

Both paths are checked. Allowing images may widen what can be *fetched*; it must never widen what
can be executed.

### The false alarm, which is worth keeping

The first run reported one survivor: `<xmp><img src=x onerror=alert(1)></xmp>`, "kept the event
handler onerror=". It had not. `<xmp>` renders its contents as literal text, so the sanitiser
escapes them, and the output was `&lt;img src=x onerror=alert(1)&gt;` — characters on a page,
never parsed as markup. The detector was scanning raw output without distinguishing a tag from
escaped text.

It is recorded because a corpus test that cries wolf is worse than none: the next real finding
gets waved through as "probably the xmp thing again". The detector now scans only inside real tag
boundaries, and `the_check_can_actually_fail` asserts both that a live handler is caught and that
escaped text is not.

### Stylesheets reach the frame now, filtered — 2026-09-22

Until this date every `<style>` element was thrown away, and the corpus's `FORBIDDEN` list says so:
`<style` may not appear in rendered output, because "a stylesheet can fetch a remote resource and
hide content". Reported from using the app: a Pi-hole daily report showed its numbers but not its
bar chart or any of its layout — its chart is a column of empty spans that only a class in a
`<style>` block gives a size and a colour. Mail styled that way, which is a large share of
generated mail, rendered as bare text.

The corpus rule still holds for the **body HTML**, which never carries a stylesheet. What changed is
a second output beside it, `Rendered.css`, which the frame puts in its `<head>`:

- **Taken out by a second ammonia parse** with an allowlist of exactly `style` and no attributes, so
  the parse that decides what *is* a stylesheet is the same html5ever parse as everything else — a
  `<style>` inside a comment, a `<textarea>` or a `<script>` is text there and text here.
- **Filtered by `src-tauri/src/mail/css.rs`**, which is now also the filter for `style` attributes,
  so the two paths cannot drift. Whole declarations go, never parts: every `url()` that is not an
  inline image, every function that takes a URL as a plain string (`image-set`, `cross-fade`,
  `image`, `src`), `@import` in every spelling, what once executed (`expression`, `javascript:`,
  `behavior`, `-moz-binding`), viewport-height units, colour-scheme queries, and anything naming
  `halcyon`. Every check runs on the text **with CSS escapes decoded** — `u\72 l(` is `url(` to the
  renderer — and whatever survives is checked once more as a whole, so the tokeniser's mistakes
  cost styling and never safety. The output never contains a `<`, so it cannot close its element.
- **Contained in the frame by cascade layers.** `halcyon-guard` is declared first and every rule in
  it is `!important`, and among important declarations the first layer wins — so no message rule,
  important or not, can undo the frame's "size by content, never scroll" rules. The one way round
  that is to *join* the layer by name, which is why the filter refuses the name.

The inline-style filter gained the same coverage in passing: before this, `image-set("https://…")`
and escape-spelled `url(` passed `style=` attributes untouched, and only the CSP stopped the load.

**Corpus: 91 payloads**, 22 of them new and all about stylesheets — escaped `url(`, comment-split
`url(`, `image-set` and friends, escaped and upper-case `@import`, `</style>` inside a string, an
attempt to write into the guard layer by name and by escape, a `100vh` root, a `<!-- -->`-wrapped
sheet. The harness checks the new output with **its own escape decoder** rather than the filter's,
so the corpus is not the filter grading its own homework, and `the_check_can_actually_fail` proves
that decoder sees an escaped `url(` and an escaped `@import`.

    XSS corpus: 91 payloads, images blocked, 0 survived
    XSS corpus: 91 payloads, images allowed, 0 survived

**One thing the design rests on, measured rather than assumed:** a frame answers
`prefers-color-scheme` from the browser's preference, not from anything set on the `<iframe>`.
`color-scheme: light` on the element was tried and changed nothing — the frame still reported dark.
So a dark-mode block in an email would switch its text white over the white card, and the filter
removes colour-scheme queries instead. `tests/e2e/messageStylesheet.spec.ts` pins the fact, so the
reason for the rule is re-examined if Chromium ever changes it.

**Not closed here:** an absolutely positioned element with a percentage height and no positioned
ancestor is sized against the frame's initial containing block, which is the frame. Inline styles
could always do that and still can; the filter has no way to see it without laying the message
out. It needs a measuring change in `MessageFrame`, not a filtering one.

---

## 2. The network trace — PASS, with a control

`tools/network-trace.ps1` records every outbound TCP connection the process opens, with reverse
DNS. Not a packet capture: it needs no administrator and answers the question the gate asks —
*which hosts did this process talk to* — rather than a question about packets. It cannot see a
connection opened and closed entirely between two 300ms polls, and that limitation is stated here
rather than left to be discovered.

**The same 18 real newsletters were opened under both settings.**

| Setting                                       | Distinct outbound connections |
| --------------------------------------------- | ----------------------------- |
| `reader.loadRemoteImages = 0` (images blocked) | **1** — Google IMAP, port 993 |
| `reader.loadRemoteImages = 1` (images loaded)  | **24** — CloudFront, Akamai, Cloudflare, Google |

Full traces: `docs/evidence/trace-images-off.txt`, `docs/evidence/trace-images-on.txt`.

With images blocked, reading eighteen marketing emails produced **no connection to any sender**.
The only socket was the mail server the account is synced against.

The second row is not a failure — it is the control, and it is what makes the first row mean
anything. Without it, "no connections observed" is equally consistent with an instrument that
cannot see connections at all. Twenty-four appearing the moment images are allowed shows the trace
works and that the setting does what it claims.

### The clause has drifted from the app, deliberately

The gate says "before consent". On 2026-08-28 remote images were switched to load **by default**,
at the owner's request, so on a default install the consent is the default rather than an act.
That is a real change to the security posture, it is documented in `PRIVACY.md` in those terms,
and it means this clause now tests the setting rather than the default. Recorded because a gate
that quietly no longer describes the app is worse than one that fails.

---

## 3. Twenty real newsletters — PARTIAL

Eighteen were opened, from the account's own mail: Google, The Economic Times, ET Wealth, The
Telegraph, Slickdeals, Groww, Booking.com, IKEA, Amazon, Skyscanner, Zomato, MakeMyTrip, VSCO,
HDFC, Uber, Nibble and others. Screenshots in both themes:
`docs/evidence/newsletter-light.png`, `docs/evidence/newsletter-dark.png`.

**Rendering, from the app's own log across 60 render events over 18 messages:**

    messages that finished rendering to empty output: 0

Zero is the number that matters. A sanitiser that ate a message would show up here as a message
with HTML in and nothing out, and none did. 481 remote images were blocked across the run.

Dark mode is correct in the way that matters for mail: the chrome inverts and the **message body
stays on a white card**. A sender's HTML carries its own colours and must not be inverted —
macOS Mail does the same.

### What this is not

It is not the twenty-message, side-by-side reading the gate describes. Eighteen messages were
opened and two were photographed; nobody has compared each against how it looks in another
client. Calling that PASS would be the same overclaim this record exists to correct.

### Found by doing it — `&shy;` in the message list

The Uber row read:

    Sign up to rider Insurance at Rs 3/trip. &shy; &shy; &shy; &shy;

Soft-hyphen entities, raw, in the preview column. Bulk senders pad with hundreds of them to push
their own text into whatever a client shows as a summary; decoded they are invisible, undecoded
they are five visible characters each.

`decode_entities` handled named and numeric entities but not `&shy;`, `&zwnj;` or `&zwj;`, and
`build_preview` never called it at all. Both fixed, with four tests — including that "Marks &
Spencer" and "R&D" survive an over-eager decoder.

**Existing rows keep their old previews.** The preview is computed when a body is fetched and
stored, so this corrects itself for a message whose body is downloaded again and not for one
already cached. A migration could clear the column; it is not obviously worth a full re-fetch.

---

## 4. Where this leaves the gate

| Clause                          | Verdict |
| -------------------------------- | ------- |
| XSS corpus blocked, output shown | PASS    |
| Network trace, no request without consent | PASS, with the caveat that consent is now the default |
| 20 newsletters, both themes      | PARTIAL — 18 opened, 0 rendered empty, 2 photographed, none compared against another client |
