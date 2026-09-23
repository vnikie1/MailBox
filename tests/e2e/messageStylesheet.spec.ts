import { expect, test, type Page } from '@playwright/test'

/**
 * A message's own stylesheet, applied in a real frame. `src/features/reader/frameDocument.ts`.
 *
 * ## Why this is an e2e spec with no app in it
 *
 * The browser build cannot show a message body — there is no core in a browser to sanitise
 * one, so it renders a placeholder — and jsdom has no layout. So this loads the frame's real
 * document builder and the real `.frame` stylesheet through Vite's module server, puts them in
 * a real `<iframe>` with the reader's sandbox, and measures what Chromium does with them.
 *
 * The CSS given to the builder here is **unfiltered**, on purpose. The core's filter
 * (`mail/css.rs`) is tested on its own; this proves the frame holds its ground even against
 * rules the filter would never have let through — apart from the one attack only the filter
 * can stop, writing into the guard's layer by name, which `css.rs` refuses and tests.
 */

interface FrameResult {
  heights: number[]
  probe: Record<string, string>
}

/** Builds a frame the way `MessageFrame` does and reports what it laid out. */
async function frame(
  page: Page,
  options: {
    html: string
    css: string
    probe: { selector: string; properties: string[] }
    /** Whether to give the iframe the reader's real `.frame` class. */
    readerClass?: boolean
    rootScheme?: 'dark' | 'light'
    /** What the selection stack passes: draw a withheld image as space, not a broken glyph. */
    hideBlocked?: boolean
  },
): Promise<FrameResult> {
  return page.evaluate(async (opts) => {
    const builder = '/src/features/reader/frameDocument.ts'
    const sheet = '/src/features/reader/MessageBody.module.css'
    const { frameDocument } = (await import(/* @vite-ignore */ builder)) as {
      frameDocument: (
        html: string,
        plainText: boolean,
        css: string,
        hideBlockedImages?: boolean,
      ) => string
    }
    const styles = (await import(/* @vite-ignore */ sheet)) as { default: { frame: string } }

    if (opts.rootScheme !== undefined) {
      document.documentElement.style.colorScheme = opts.rootScheme
    }

    const element = document.createElement('iframe')
    element.setAttribute('sandbox', 'allow-same-origin')
    if (opts.readerClass === true) element.className = styles.default.frame
    element.style.width = '600px'
    element.style.height = '60px'
    element.style.border = '0'

    const loaded = new Promise<void>((resolve) => {
      element.addEventListener('load', () => {
        resolve()
      })
    })
    element.srcdoc = frameDocument(opts.html, false, opts.css, opts.hideBlocked === true)
    document.body.append(element)
    await loaded

    const doc = element.contentDocument
    if (doc === null) throw new Error('no document in the frame')

    // What `MessageFrame`'s `measure` does, run to a fixed point by hand: set the frame to the
    // document's height and read it again. A document sized against its frame never settles.
    const heights: number[] = []
    for (let round = 0; round < 8; round += 1) {
      const next = doc.documentElement.scrollHeight
      heights.push(next)
      element.style.height = `${String(next)}px`
      await new Promise((resolve) => requestAnimationFrame(resolve))
    }

    const target = doc.querySelector(opts.probe.selector)
    if (target === null) throw new Error(`nothing matches ${opts.probe.selector}`)
    const computed = doc.defaultView?.getComputedStyle(target)

    const probe: Record<string, string> = {}
    for (const property of opts.probe.properties) {
      probe[property] = computed?.getPropertyValue(property) ?? ''
    }

    element.remove()
    return { heights, probe }
  }, options)
}

/** The Pi-hole daily report's shape: its whole chart is empty spans and a class. */
const REPORT_CSS =
  '.bar{display:inline-block;height:8px;background:#0071e3;border-radius:4px;vertical-align:middle}'
const REPORT_HTML =
  '<table><tr><td>100.64.0.1</td><td><span class="bar" style="width:120px"></span></td></tr></table>'

test.describe('a message stylesheet in the reader frame', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible()
  })

  test('draws the bars of a chart made of nothing but a class', async ({ page }) => {
    const { probe } = await frame(page, {
      html: REPORT_HTML,
      css: REPORT_CSS,
      probe: { selector: '.bar', properties: ['display', 'height', 'width', 'background-color'] },
    })

    expect(probe).toEqual({
      display: 'inline-block',
      height: '8px',
      width: '120px',
      'background-color': 'rgb(0, 113, 227)',
    })
  })

  test('without the stylesheet the same bar is nothing at all', async ({ page }) => {
    // The bug as it was reported, so this spec can tell the fix from no fix: an inline span
    // ignores its width, has no height, and paints nothing.
    const { probe } = await frame(page, {
      html: REPORT_HTML,
      css: '',
      probe: { selector: '.bar', properties: ['display', 'background-color'] },
    })

    expect(probe.display).toBe('inline')
    expect(probe['background-color']).toBe('rgba(0, 0, 0, 0)')
  })

  test('lets a message style its own page', async ({ page }) => {
    // The frame's defaults are meant to lose to the sender's: the report puts its grey page and
    // its padding on <body>, and a frame that kept its own would show neither.
    const { probe } = await frame(page, {
      html: '<p>report</p>',
      css: 'body{padding:24px;background:#f5f5f7}',
      probe: { selector: 'body', properties: ['padding-top', 'background-color'] },
    })

    expect(probe['padding-top']).toBe('24px')
    expect(probe['background-color']).toBe('rgb(245, 245, 247)')
  })

  test('settles at its content height when a message sizes its root against the frame', async ({
    page,
  }) => {
    // `height: 100%` on html and body is boilerplate in a great deal of email. With padding on
    // top, a root that fills its frame is taller than its frame, and a frame sized to fit that
    // grows by the padding every time it is measured.
    const { heights, probe } = await frame(page, {
      html: '<p>short</p>',
      css: 'html,body{height:100%!important;min-height:100%!important;padding:24px}html{overflow:auto!important}',
      probe: { selector: 'html', properties: ['overflow-y', 'height'] },
    })

    const last = heights[heights.length - 1] ?? Number.NaN
    const before = heights[heights.length - 2] ?? Number.NaN

    expect(last, `heights: ${heights.join(', ')}`).toBe(before)
    expect(last, `heights: ${heights.join(', ')}`).toBeLessThan(200)
    expect(probe['overflow-y']).toBe('hidden')
  })

  test('draws a withheld image as space for the stack, and as a glyph for the reader', async ({
    page,
  }) => {
    // `blocked:remote` is not a real scheme, so a browser draws its broken-image glyph. In the
    // reader that is right — a banner above says what is missing and offers to load it. The
    // selection stack has neither, and refuses remote images on purpose, so a page of broken
    // glyphs there reads as the app failing rather than declining.
    const html = '<img src="blocked:remote" width="200" height="60">'
    const probe = { selector: 'img', properties: ['visibility', 'width', 'height'] }

    const stack = await frame(page, { html, css: '', probe, hideBlocked: true })
    expect(stack.probe.visibility).toBe('hidden')

    // The space it was given stays its size, so nothing reflows around the gap.
    expect(stack.probe.width).toBe('200px')
    expect(stack.probe.height).toBe('60px')

    const reader = await frame(page, { html, css: '', probe })
    expect(reader.probe.visibility).toBe('visible')
  })

  test('stays a light card when the app and the message both ask for dark', async ({ page }) => {
    // A message can declare `color-scheme: dark` on its root, which turns the system colours
    // dark: CanvasText would become white, and the frame paints white. The guard pins it.
    await page.emulateMedia({ colorScheme: 'dark' })

    const { probe } = await frame(page, {
      html: '<p>legible</p>',
      css: ':root{color-scheme:dark}p{color:CanvasText}',
      probe: { selector: 'p', properties: ['color'] },
      readerClass: true,
      rootScheme: 'dark',
    })

    expect(probe.color).toBe('rgb(0, 0, 0)')
  })

  test('answers prefers-color-scheme from the browser, which is why the core drops those blocks', async ({
    page,
  }) => {
    // Not a rule of the frame's; a fact about Chromium the design rests on, pinned so that if it
    // ever changes the reason for `mail/css.rs` refusing colour-scheme queries is re-examined
    // rather than silently outlived. `color-scheme: light` on the <iframe> element was tried
    // first and changes nothing: the frame still reports dark.
    await page.emulateMedia({ colorScheme: 'dark' })

    const { probe } = await frame(page, {
      html: '<p>legible</p>',
      css: '@media (prefers-color-scheme: dark){p{color:rgb(255, 255, 255)}}',
      probe: { selector: 'p', properties: ['color'] },
      readerClass: true,
      rootScheme: 'dark',
    })

    expect(probe.color).toBe('rgb(255, 255, 255)')
  })
})
