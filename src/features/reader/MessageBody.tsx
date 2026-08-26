import { useCallback, useEffect, useRef, useState } from 'react'
import { ImageOff } from 'lucide-react'

import { openExternal } from '@/lib/ipc'
import { useMessageBody } from '@/app/queries'
import { Button } from '@/ui'

import styles from './MessageBody.module.css'

/**
 * A message body, rendered. docs/03-architecture.md §6.
 *
 * The frame is `sandbox="allow-same-origin"` and **nothing else** — in particular no
 * `allow-scripts`, so nothing in a message can execute. That has one consequence worth
 * spelling out, because it looks like a contradiction with §6.7's "post `scrollHeight` on
 * load": the message cannot post anything, because it cannot run. `allow-same-origin` is what
 * lets *this* component reach into the frame and read the height itself. The measuring code
 * is ours, on our side of the boundary, and the message is inert.
 *
 * Everything the frame is allowed to load is already inside the document it is given: inline
 * images arrived as `data:` URIs from the local cache, and remote images either were fetched
 * by the Rust core or are not there at all. The CSP says so as well, so a mistake in the
 * sanitiser still cannot become a network request.
 */

/** docs/03 §6.5, verbatim. */
const CSP = "default-src 'none'; img-src cid: app: data:; style-src 'unsafe-inline';"

/**
 * Styling for the frame's document, not the app's.
 *
 * It cannot use the token layer: the frame is a separate document and CSS custom properties
 * do not cross that boundary. The colours are passed in from the resolved theme instead, so
 * a message still reads correctly in dark mode — mail is overwhelmingly written for a white
 * background, so the default is a light card even in dark mode, which is what Mail does.
 */
function frameDocument(html: string, plainText: boolean): string {
  return `<!doctype html>
<html><head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${CSP}">
<base target="_blank">
<style>
  html, body { margin: 0; padding: 0; background: #fff; color: #1c1c1e; }
  body {
    font: 14px/1.55 -apple-system, "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
    padding: 4px 2px 16px;
    word-break: break-word;
    overflow-wrap: anywhere;
  }
  img { max-width: 100%; height: auto; }
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
  ${plainText ? '' : 'body > :first-child { margin-top: 0; }'}
</style>
</head><body>${html}</body></html>`
}

export interface MessageBodyProps {
  messageId: number
  className?: string | undefined
}

export function MessageBody({ messageId, className }: MessageBodyProps) {
  const frameRef = useRef<HTMLIFrameElement | null>(null)
  const [loadRemote, setLoadRemote] = useState(false)
  const [height, setHeight] = useState(0)

  // Consent is per message and never remembered. Someone who wanted to see one newsletter's
  // pictures has not agreed to tell every future sender when their mail was opened.
  useEffect(() => {
    setLoadRemote(false)
    setHeight(0)
  }, [messageId])

  // A query, not an effect, so it is invalidated by `messages:updated` when the body
  // finishes downloading. Bodies are fetched lazily *after* selection, so the first render
  // of a message that has just been clicked legitimately has nothing to show — and without
  // the invalidation it would go on showing nothing until the user clicked away and back.
  const { data: rendered, isPending } = useMessageBody(messageId, loadRemote)

  const hasContent = rendered !== undefined && rendered.html.trim() !== ''
  const empty = rendered !== undefined && !hasContent

  /**
   * Measures the frame and wires up its links.
   *
   * Runs after every render of the document because the height is only knowable once images
   * have laid out, and a frame sized before that clips the message.
   */
  const onFrameLoad = useCallback(() => {
    const frame = frameRef.current
    const doc = frame?.contentDocument
    if (!frame || !doc) return

    const measure = () => {
      const next = doc.documentElement.scrollHeight
      // A one-pixel jitter loop is possible when a scrollbar appears and disappears; only
      // grow, and only past a threshold.
      setHeight((current) => (next > current + 1 ? next : current))
    }

    measure()

    // Images finish after load, and each one changes the height.
    doc.querySelectorAll('img').forEach((image) => {
      if (!image.complete) image.addEventListener('load', measure, { once: true })
    })

    // Wide tables get their own scroller rather than forcing the whole message sideways.
    doc.querySelectorAll('table').forEach((table) => {
      if (table.scrollWidth > doc.documentElement.clientWidth) {
        table.classList.add('halcyon-scroll')
      }
    })

    // docs/03 §6.6 — links open in the default browser, never in the WebView. Intercepted
    // here rather than trusted to `target="_blank"`, because in a WebView that would open a
    // second WebView with no address bar, which is the worst of both.
    doc.addEventListener('click', (event) => {
      const anchor = (event.target as Element | null)?.closest('a')
      const href = anchor?.getAttribute('href')
      if (href === null || href === undefined) return

      event.preventDefault()
      if (/^https?:/i.test(href) || href.startsWith('mailto:')) {
        void openExternal(href, anchor?.textContent ?? '')
      }
    })

    const observer = new ResizeObserver(measure)
    observer.observe(doc.documentElement)

    frame.addEventListener('beforeunload', () => {
      observer.disconnect()
    })
  }, [])

  // Nothing to show yet. An empty white card in place of a message reads as the app being
  // broken; a line saying what is happening reads as the app working.
  if (isPending || empty) {
    return (
      <div className={className}>
        <p className={styles.pending}>
          {isPending ? 'Loading message…' : 'Downloading this message…'}
        </p>
      </div>
    )
  }

  if (rendered === undefined) return <div className={className} />

  return (
    <div className={className}>
      {rendered.blockedRemote > 0 && (
        <div className={styles.banner} role="status">
          <ImageOff className={styles.bannerIcon} aria-hidden />
          <span className={styles.bannerText}>
            {rendered.blockedRemote === 1
              ? '1 remote image was not loaded.'
              : `${String(rendered.blockedRemote)} remote images were not loaded.`}{' '}
            Loading them tells the sender you opened this message.
          </span>
          <Button
            variant="bordered"
            onClick={() => {
              setLoadRemote(true)
            }}
          >
            Load Images
          </Button>
        </div>
      )}

      <iframe
        ref={frameRef}
        title="Message content"
        className={styles.frame}
        // No allow-scripts, no allow-popups, no allow-top-navigation. docs/03 §6.1.
        sandbox="allow-same-origin"
        srcDoc={frameDocument(rendered.html, rendered.fromPlainText)}
        style={height > 0 ? { height: `${String(height)}px` } : undefined}
        onLoad={onFrameLoad}
      />
    </div>
  )
}
