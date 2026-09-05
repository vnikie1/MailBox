import { useEffect, useRef, useState } from 'react'

import type { PDFDocumentLoadingTask, PDFDocumentProxy } from 'pdfjs-dist'

import styles from './PdfView.module.css'

/**
 * A PDF, drawn by the app rather than by the WebView.
 *
 * Every other attachment is shown in a sandboxed frame, and a PDF cannot be: Chromium's PDF
 * viewer is a *plugin*, and a plugin refuses to load into a sandboxed frame — "Failed to load
 * … as a plugin, because the frame into which the plugin is loading is sandboxed". Dropping
 * the sandbox is not on the table, because an attachment is exactly as hostile as a message
 * body. So the previewer used to say PDFs were not supported, which is a poor answer for the
 * most common thing anyone attaches to an email.
 *
 * pdf.js resolves that without giving anything away. It parses the file in JavaScript and
 * paints pixels onto a `<canvas>`: no plugin, no navigation, no scripting of the PDF's own,
 * and nothing that can reach the network or the IPC bridge. The bytes never become a
 * document. It is bundled rather than fetched, so the previewer works offline and pulls
 * nothing at runtime.
 *
 * Loaded on demand. pdf.js and its worker are about 1.7MB, and a cold start that pays for a
 * PDF renderer nobody opened would spend a good part of the startup budget in docs/06 on it.
 */

export interface PdfViewProps {
  /** The file's bytes. Copied per load, because pdf.js transfers the buffer to its worker. */
  bytes: Uint8Array
  filename: string
}

/** The size of a page at scale 1, used to reserve its space before it is drawn. */
interface PageShape {
  width: number
  height: number
}

/**
 * pdf.js and its worker.
 *
 * The worker arrives as a real same-origin URL rather than a blob, which matters: the app's
 * CSP is `script-src 'self'`, and pdf.js's blob fallback would be blocked by it. What happens
 * on that path is not an error but a "fake worker" that parses on the main thread and freezes
 * the window, so it is worth being explicit about.
 */
async function loadPdfjs() {
  const [pdfjs, worker] = await Promise.all([
    import('pdfjs-dist'),
    import('pdfjs-dist/build/pdf.worker.min.mjs?url'),
  ])

  pdfjs.GlobalWorkerOptions.workerSrc = worker.default
  return pdfjs
}

/** The data pdf.js fetches at runtime. See the `pdfjsData` plugin in `vite.config.ts`. */
const DATA_URLS = {
  standardFontDataUrl: `${import.meta.env.BASE_URL}pdfjs/standard_fonts/`,
  cMapUrl: `${import.meta.env.BASE_URL}pdfjs/cmaps/`,
  cMapPacked: true,
}

export function PdfView({ bytes, filename }: PdfViewProps) {
  // Held in state rather than a ref because the pages observe it: a ref's `current` is not a
  // render input, so pages mounted on the first pass would observe `null` and never be told.
  const [tray, setTray] = useState<HTMLDivElement | null>(null)
  const [document, setDocument] = useState<PDFDocumentProxy | null>(null)
  const [shapes, setShapes] = useState<PageShape[]>([])
  const [error, setError] = useState<string | null>(null)
  const [width, setWidth] = useState(0)

  // The tray's content width drives the render scale, so a page is drawn at the size it is
  // shown at rather than drawn once and resampled by the browser.
  useEffect(() => {
    if (tray === null) return

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) setWidth(entry.contentRect.width)
    })

    observer.observe(tray)
    return () => {
      observer.disconnect()
    }
  }, [tray])

  useEffect(() => {
    let cancelled = false
    // The loading task, not the document: `destroy` lives there in pdf.js 6, and it is what
    // tears down the worker as well as the document.
    let task: PDFDocumentLoadingTask | null = null

    setDocument(null)
    setShapes([])
    setError(null)

    // Loading and committing are split so that each `cancelled` check sits in its own
    // function, which is the only thing that stops the check below it reading as dead code.
    const load = async () => {
      const pdfjs = await loadPdfjs()
      // Copied, because `getDocument` transfers the buffer to the worker and detaches it.
      // Without this, opening the same attachment twice fails on an empty ArrayBuffer.
      task = pdfjs.getDocument({ data: new Uint8Array(bytes), ...DATA_URLS })

      const document = await task.promise

      // Every page measured up front so the scroller has its real height immediately.
      // Reserving space page by page as each one renders would move the content under the
      // reader's finger while they scroll.
      const measured = await Promise.all(
        Array.from({ length: document.numPages }, (_unused, index) =>
          document.getPage(index + 1).then((page) => {
            const viewport = page.getViewport({ scale: 1 })
            return { width: viewport.width, height: viewport.height }
          }),
        ),
      )

      return { document, measured }
    }

    void load()
      .then(({ document, measured }) => {
        // Not merely an unmount guard: switching attachments destroys this document, and
        // showing a destroyed one is worse than showing nothing.
        if (cancelled) return

        setShapes(measured)
        setDocument(document)
      })
      .catch((cause: unknown) => {
        // Destroying the task rejects whatever was still in flight, so a cancelled load
        // arrives here as a failure and is not one.
        if (cancelled) return

        // A password prompt is a different thing from a broken file, and saying "could not be
        // opened" for an encrypted PDF sends the reader looking for a fault that is not there.
        const name = cause instanceof Error ? cause.name : ''
        setError(
          name === 'PasswordException'
            ? 'This PDF is password-protected. Save it to open it in your PDF reader.'
            : 'This PDF could not be opened. It may be damaged or incomplete.',
        )
      })

    return () => {
      cancelled = true
      void task?.destroy()
    }
  }, [bytes])

  if (error !== null) {
    return (
      <div className={styles.status}>
        <p>{error}</p>
      </div>
    )
  }

  return (
    <div ref={setTray} className={styles.tray}>
      {document === null ? (
        <p className={styles.loading}>Opening {filename}…</p>
      ) : (
        <div className={styles.pages}>
          {shapes.map((shape, index) => (
            <PdfPage
              key={index}
              document={document}
              pageNumber={index + 1}
              shape={shape}
              width={width}
              root={tray}
            />
          ))}
        </div>
      )}
    </div>
  )
}

/**
 * One page, drawn only while it is on or near the screen.
 *
 * The core allows a preview up to 16MB, which is a few hundred pages of text. Drawing all of
 * them at once is gigabytes of canvas; drawing them as they are reached costs nothing for the
 * pages nobody looks at, and gives back the memory for the ones they scroll past.
 */
function PdfPage({
  document,
  pageNumber,
  shape,
  width,
  root,
}: {
  document: PDFDocumentProxy
  pageNumber: number
  shape: PageShape
  width: number
  root: HTMLElement | null
}) {
  const hostRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [near, setNear] = useState(false)

  useEffect(() => {
    const host = hostRef.current
    if (host === null) return

    // Against the tray, not the viewport. With the default root the margin below buys nothing:
    // a page scrolled out of the tray is *clipped* by it, so its visible area is zero however
    // far the viewport rect is grown, and pages would blank out at the edge of the scroll
    // rather than be drawn ahead of it. Two trays of margin either way, so scrolling at a
    // normal pace never catches a page mid-draw.
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) setNear(entry.isIntersecting)
      },
      { root, rootMargin: '200% 0px' },
    )

    observer.observe(host)
    return () => {
      observer.disconnect()
    }
  }, [root])

  const scale = width > 0 ? width / shape.width : 0

  useEffect(() => {
    const canvas = canvasRef.current
    if (!near || canvas === null || scale === 0) return

    let cancelled = false
    let task: { cancel: () => void } | null = null

    void document
      .getPage(pageNumber)
      .then((page) => {
        if (cancelled) return

        // Capped at 2: a scanned A4 page at the full ratio of a 4K display is a
        // 60-megapixel canvas, and past 2 the difference is invisible in a preview.
        const ratio = Math.min(window.devicePixelRatio || 1, 2)
        const viewport = page.getViewport({ scale: scale * ratio })

        canvas.width = Math.floor(viewport.width)
        canvas.height = Math.floor(viewport.height)

        const render = page.render({ canvas, viewport })
        task = render
        return render.promise
      })
      .catch((cause: unknown) => {
        // Cancelling is how this component stops work it no longer needs, so the rejection
        // that raises is expected rather than a failure worth reporting.
        const name = cause instanceof Error ? cause.name : ''
        if (name !== 'RenderingCancelledException') throw cause
      })

    return () => {
      cancelled = true
      task?.cancel()
      // Zeroing the canvas releases its backing store. Leaving it sized holds that memory for
      // as long as the element lives, which defeats the point of drawing lazily at all.
      canvas.width = 0
      canvas.height = 0
    }
  }, [document, pageNumber, scale, near])

  return (
    <div
      ref={hostRef}
      className={styles.page}
      // Reserved from the page's own proportions so the scrollbar is honest before anything
      // is drawn, and does not jump as pages arrive.
      style={{ aspectRatio: `${shape.width} / ${shape.height}` }}
    >
      <canvas ref={canvasRef} className={styles.canvas} aria-label={`Page ${pageNumber}`} />
    </div>
  )
}
