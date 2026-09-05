import { useEffect, useMemo, useState } from 'react'

import type { AttachmentData } from '@/lib/generated/AttachmentData'
import { attachmentPreview, attachmentSave } from '@/lib/ipc'
import { Button, Sheet, useToast } from '@/ui'

import styles from './AttachmentPreview.module.css'
import { PdfView } from './PdfView'

/**
 * The built-in previewer. docs/04 Phase 6.
 *
 * Built in rather than handing the file to the shell, and that is a security decision rather
 * than a convenience one — `ipc/attachments.rs` sets out the reasoning. The short version: an
 * "Open" button beside a file called `invoice.pdf.exe` is a loaded gun with a friendly label.
 *
 * Nothing shown here is ever handed to the platform. Images and text are drawn by the WebView
 * inside a frame that cannot script; PDFs are parsed and painted onto a canvas by pdf.js, which
 * is bundled (see `PdfView.tsx` for why a frame cannot do it). Either way the content type was
 * one the **core** decided was safe. When the core refuses, the only thing offered is Save —
 * and the shell's own warnings stay intact when the user opens it themselves, which is where
 * that decision belongs.
 *
 * Built on `Sheet` rather than a hand-rolled modal so it inherits the focus trap, the Escape
 * and click-outside dismissal, and the house transition. A second modal implementation is a
 * second set of accessibility bugs.
 */

export interface AttachmentPreviewProps {
  attachmentId: number
  filename: string
  onClose: () => void
}

/** The frame's CSP. `data:` only, and no script under any circumstances. */
const CSP = "default-src 'none'; img-src data:; style-src 'unsafe-inline';"

/**
 * The bytes of an attachment the core sent as a `data:` URI.
 *
 * `atob` yields one byte per character, so the bytes are rebuilt rather than read as text:
 * decoding the string directly would mangle every character outside ASCII, and a PDF is not
 * text at all.
 */
function bytesOf(dataUrl: string): Uint8Array {
  const base64 = dataUrl.slice(dataUrl.indexOf(',') + 1)

  try {
    const binary = atob(base64)
    return Uint8Array.from(binary, (character) => character.charCodeAt(0))
  } catch {
    // A malformed data URI is the core's bug, not something to crash the reader over.
    return new Uint8Array()
  }
}

/**
 * The text of an attachment the core sent as a `data:` URI.
 *
 * Decoded here rather than handed to the frame as a URI, and that is the fix rather than a
 * refinement. Text used to be shown with `<object data="data:…">`, and **every non-image
 * preview came up blank**: the preview frame carries `sandbox`, so it inherits the application's
 * own Content-Security-Policy, and that says `object-src 'none'`. Policies combine
 * most-restrictive-wins, so the `object-src data:` in the frame's own policy above could never
 * take effect. Chromium says so in as many words — "Loading plugin data from 'data:…' violates
 * the following Content Security Policy directive: object-src 'none'" — but nothing surfaces a
 * console message from inside a frame, so it presented as an empty modal.
 *
 * Drawing the text as text needs no plugin, no `object`, and no widening of any policy.
 */
function textOf(dataUrl: string): string {
  return new TextDecoder().decode(bytesOf(dataUrl))
}

/** Escapes text for the frame. An attachment is hostile input; this one is being shown as text. */
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

function frameDocument(data: AttachmentData): string {
  const mime = data.mime.toLowerCase()

  // The `data:` URI is built by the core and is base64 of bytes it decoded, so it cannot
  // carry a quote that would break out of the attribute. It goes in an attribute rather than
  // as markup either way, and the CSP above allows no scripting even if it did.
  const body = mime.startsWith('image/')
    ? `<img src="${data.dataUrl}" alt="">`
    : `<pre>${escapeHtml(textOf(data.dataUrl))}</pre>`

  return `<!doctype html>
<html><head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${CSP}">
<style>
  html, body { margin: 0; padding: 0; height: 100%; background: #fff; color: #1c1c1e; }
  body { display: grid; place-items: center; }
  img { max-width: 100%; max-height: 100%; object-fit: contain; }
  pre {
    place-self: stretch;
    margin: 0;
    padding: 12px 14px;
    overflow: auto;
    font: 12px/1.5 "Cascadia Mono", Consolas, ui-monospace, monospace;
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
</head><body>${body}</body></html>`
}

export function AttachmentPreview({ attachmentId, filename, onClose }: AttachmentPreviewProps) {
  const [data, setData] = useState<AttachmentData | null>(null)
  const [error, setError] = useState<string | null>(null)
  const toast = useToast()

  // Held stable across renders because `PdfView` reloads the document whenever these change,
  // and decoding a 16MB attachment on every render would be its own bug.
  const pdfBytes = useMemo(
    () =>
      data !== null && data.mime.toLowerCase() === 'application/pdf' ? bytesOf(data.dataUrl) : null,
    [data],
  )

  useEffect(() => {
    let cancelled = false
    setData(null)
    setError(null)

    attachmentPreview(attachmentId)
      .then((result) => {
        if (!cancelled) setData(result)
      })
      .catch((cause: unknown) => {
        if (cancelled) return
        // The core's own message, not a generic one: it distinguishes "too large to preview"
        // from "this kind of file cannot be shown here", and those need different responses
        // from the reader.
        const message =
          typeof cause === 'object' && cause !== null && 'message' in cause
            ? String(cause.message)
            : 'This attachment could not be opened.'
        setError(message)
      })

    return () => {
      cancelled = true
    }
  }, [attachmentId])

  return (
    <Sheet
      open
      onOpenChange={(next) => {
        if (!next) onClose()
      }}
      title={filename}
      className={styles.sheet}
      footer={
        <>
          <Button
            variant="bordered"
            onClick={() => {
              // The result is not discardable. `null` means the user closed the file dialog,
              // which deserves silence -- but a rejection is a real failure the core has
              // already worded ("The file could not be saved to that location", for a full
              // disk, a read-only folder or a disconnected drive), and throwing it away left
              // Save… looking like it had worked. The file simply was not there afterwards.
              attachmentSave(attachmentId)
                .then((saved) => {
                  if (saved !== null) toast.show({ title: `Saved ${filename}` })
                })
                .catch((cause: unknown) => {
                  toast.show({
                    title: 'The attachment could not be saved',
                    description: cause instanceof Error ? cause.message : String(cause),
                  })
                })
            }}
          >
            Save…
          </Button>
          <Button variant="filled" onClick={onClose}>
            Done
          </Button>
        </>
      }
    >
      {error !== null ? (
        <div className={styles.message}>
          <p>{error}</p>
        </div>
      ) : data === null ? (
        <div className={styles.message}>
          <p>Opening…</p>
        </div>
      ) : pdfBytes !== null ? (
        // Drawn by the app, not by the frame. Chromium's PDF viewer is a plugin and refuses to
        // load into a sandboxed frame, and the sandbox is not negotiable for an attachment, so
        // a PDF here is rendered by pdf.js onto a canvas instead. `PdfView` has the detail.
        <PdfView bytes={pdfBytes} filename={filename} />
      ) : (
        <iframe
          title={filename}
          className={styles.frame}
          // No allow-scripts. An attachment is exactly as hostile as a message body.
          sandbox="allow-same-origin"
          srcDoc={frameDocument(data)}
        />
      )}
    </Sheet>
  )
}
