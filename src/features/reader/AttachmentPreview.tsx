import { useEffect, useState } from 'react'

import type { AttachmentData } from '@/lib/generated/AttachmentData'
import { attachmentPreview, attachmentSave } from '@/lib/ipc'
import { Button, Sheet } from '@/ui'

import styles from './AttachmentPreview.module.css'

/**
 * The built-in previewer. docs/04 Phase 6.
 *
 * Built in rather than handing the file to the shell, and that is a security decision rather
 * than a convenience one — `ipc/attachments.rs` sets out the reasoning. The short version: an
 * "Open" button beside a file called `invoice.pdf.exe` is a loaded gun with a friendly label.
 *
 * Everything shown here is rendered by the WebView from a `data:` URI, inside a frame that
 * cannot script, for a content type the **core** decided was safe. When the core refuses, the
 * only thing offered is Save — and the shell's own warnings stay intact when the user opens
 * it themselves, which is where that decision belongs.
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
const CSP = "default-src 'none'; img-src data:; object-src data:; style-src 'unsafe-inline';"

function frameDocument(data: AttachmentData): string {
  const mime = data.mime.toLowerCase()

  // The `data:` URI is built by the core and is base64 of bytes it decoded, so it cannot
  // carry a quote that would break out of the attribute. It goes in an attribute rather than
  // as markup either way, and the CSP above allows no scripting even if it did.
  const body = mime.startsWith('image/')
    ? `<img src="${data.dataUrl}" alt="">`
    : mime === 'application/pdf'
      ? `<object data="${data.dataUrl}" type="application/pdf"></object>`
      : `<object data="${data.dataUrl}" type="text/plain"></object>`

  return `<!doctype html>
<html><head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${CSP}">
<style>
  html, body { margin: 0; padding: 0; height: 100%; background: #fff; }
  body { display: grid; place-items: center; }
  img { max-width: 100%; max-height: 100%; object-fit: contain; }
  object { width: 100%; height: 100%; border: 0; }
</style>
</head><body>${body}</body></html>`
}

export function AttachmentPreview({ attachmentId, filename, onClose }: AttachmentPreviewProps) {
  const [data, setData] = useState<AttachmentData | null>(null)
  const [error, setError] = useState<string | null>(null)

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
              void attachmentSave(attachmentId)
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
