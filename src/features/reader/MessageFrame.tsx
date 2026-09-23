import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { openExternal } from '@/lib/ipc'

import { frameDocument } from './frameDocument'
import { repairShortRows } from './repairTables'

import styles from './MessageBody.module.css'

/**
 * The sandboxed frame a message body is displayed in. docs/03 §6.
 *
 * Extracted from `MessageBody` so that the `.eml` viewer window can show a message without a
 * second implementation of this. That is not tidiness: this component is the security boundary
 * standing rule 11 is about, and two copies of a boundary means one of them is eventually the
 * one that drifts — and the drifted copy is not obviously wrong to read.
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

export interface MessageFrameProps {
  /** Sanitised by the Rust core. There is no path to this component that skips that. */
  html: string
  /**
   * The message's own stylesheet, filtered by the core. Empty for most mail, and for every
   * message rendered from plain text. See `frameDocument` for where it goes and why.
   */
  css: string
  /** Set when the core built the HTML from a plain-text part, which changes the first margin. */
  fromPlainText: boolean
  /** Resets the measured height when the frame is given a different message. */
  resetKey?: string | number | undefined
  /**
   * Draws a withheld remote image as empty space rather than as a broken-image glyph.
   *
   * For a frame with no banner over it to explain the gap and offer to fill it — the selection
   * stack, which refuses remote images whatever the setting. The reader keeps the glyph.
   */
  hideBlockedImages?: boolean | undefined
}

export function MessageFrame({
  html,
  css,
  fromPlainText,
  resetKey,
  hideBlockedImages = false,
}: MessageFrameProps) {
  const frameRef = useRef<HTMLIFrameElement | null>(null)

  // Rows missing their trailing cells, repaired before the document is built. Memoised because
  // it parses the markup, and the markup only changes when a different message is shown —
  // without this it would re-parse on every resize, and resizing is what this component does.
  const repaired = useMemo(
    () => (fromPlainText ? html : repairShortRows(html)),
    [html, fromPlainText],
  )

  /** The observer watching the document currently in the frame, so it can be replaced. */
  const observers = useRef<ResizeObserver | null>(null)
  const [height, setHeight] = useState(0)

  // A new message starts from nothing. Keeping the previous height would leave the frame the
  // size of the message before it, which is visible as a jump on every selection.
  useEffect(() => {
    setHeight(0)
  }, [resetKey])

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

    /**
     * Re-measures allowing the frame to *shrink*.
     *
     * Only ever called from a real user action — closing the quoted-text fold — because that
     * is the one case where the document legitimately gets shorter. `measure` refuses to
     * shrink on purpose, since a spontaneous shrink is almost always scrollbar jitter, and
     * following it produces a frame that oscillates.
     */
    const remeasure = () => {
      // After the fold's own layout, not during it.
      requestAnimationFrame(() => {
        setHeight(doc.documentElement.scrollHeight)
      })
    }

    measure()

    // The core folds quoted replies into a <details>. The frame runs no script, so nothing
    // inside can tell us it opened — this side has to listen for it.
    doc.querySelectorAll('details').forEach((fold) => {
      fold.addEventListener('toggle', remeasure)
    })

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

      // A data detector — a tracking number or a phone number the core recognised. These
      // are not links the sender wrote, so they resolve to an action rather than to a URL,
      // and the action still goes out through `openExternal` like any other.
      const detected = anchor?.getAttribute('data-detected')
      if (detected !== null && detected !== undefined) {
        const value = anchor?.getAttribute('data-value') ?? ''
        const target =
          detected === 'phone'
            ? `tel:${value.replace(/[^\d+]/g, '')}`
            : // Searched rather than sent to one carrier's site: the format says which
              // carrier it probably is, not which it certainly is, and guessing wrong sends
              // the user to a page that says the parcel does not exist.
              `https://www.google.com/search?q=${encodeURIComponent(value)}`

        void openExternal(target, value)
        return
      }

      if (/^https?:/i.test(href) || href.startsWith('mailto:')) {
        void openExternal(href, anchor?.textContent ?? '')
      }
    })

    // Torn down explicitly, and the previous one with it.
    //
    // This used to be `frame.addEventListener('beforeunload', …)`, which never fired:
    // `beforeunload` is a Window event and `frame` is the iframe *element*, so the listener sat
    // on something that would never raise it. Every message opened left an observer watching a
    // document that had been replaced — the sort of thing that is invisible until a long
    // session and impossible to attribute afterwards.
    observers.current?.disconnect()

    const observer = new ResizeObserver(measure)
    observer.observe(doc.documentElement)
    observers.current = observer
  }, [])

  // And on the way out, where nothing was watching at all.
  useEffect(
    () => () => {
      observers.current?.disconnect()
      observers.current = null
    },
    [],
  )

  return (
    <iframe
      ref={frameRef}
      title="Message content"
      className={styles.frame}
      // No allow-scripts, no allow-popups, no allow-top-navigation. docs/03 §6.1.
      sandbox="allow-same-origin"
      srcDoc={frameDocument(repaired, fromPlainText, css, hideBlockedImages)}
      style={height > 0 ? { height: `${String(height)}px` } : undefined}
      onLoad={onFrameLoad}
    />
  )
}
