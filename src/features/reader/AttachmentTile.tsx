import {
  Download,
  File,
  FileArchive,
  FileAudio,
  FileImage,
  FileSpreadsheet,
  FileText,
  FileType,
  FileVideo,
  Presentation,
  type LucideIcon,
} from 'lucide-react'

import type { AttachmentRow } from '@/lib/generated/AttachmentRow'
import { formatFileSize } from '@/lib/date'
import { IconButton } from '@/ui'

import styles from './AttachmentTile.module.css'

/**
 * One attachment, as a card with its own icon and a way to save it.
 *
 * ## Why a card rather than the row it was
 *
 * It used to be the 44px pill docs/02 §6.8 specifies: a paperclip, a name, a size. That is a
 * deviation now, and deliberately — the spec describes a *list item*, and this is the thing at
 * the bottom of a message that a reader reaches for. Two problems with the row in practice:
 * every attachment looked identical, because a paperclip says only "attachment"; and saving one
 * meant opening the preview first and finding the button in there.
 *
 * The card names the kind of file before the filename is read, which is most of what someone
 * scanning a message wants to know, and puts saving one click from the message.
 *
 * ## The icon is chosen from the MIME type, and falls back twice
 *
 * The type the sender declared first, then the extension, then a plain document. Senders get
 * MIME types wrong constantly — `application/octet-stream` for a PDF is routine — so the
 * extension is a real second opinion rather than a formality.
 */
export interface AttachmentTileProps {
  attachment: AttachmentRow
  /** Opens the built-in preview. The card itself is the trigger. */
  onOpen: () => void
  /** Saves it, asking where. */
  onSave: () => void
}

/** MIME prefix or exact type to icon, in the order it is tried. */
const BY_MIME: [string, LucideIcon][] = [
  ['application/pdf', FileType],
  ['image/', FileImage],
  ['video/', FileVideo],
  ['audio/', FileAudio],
  ['text/', FileText],
  ['application/zip', FileArchive],
  ['application/x-7z', FileArchive],
  ['application/vnd.rar', FileArchive],
  ['application/gzip', FileArchive],
  ['message/rfc822', FileText],
]

/** Extension to icon, consulted when the declared type says nothing useful. */
const BY_EXTENSION: Record<string, LucideIcon> = {
  pdf: FileType,
  png: FileImage,
  jpg: FileImage,
  jpeg: FileImage,
  gif: FileImage,
  webp: FileImage,
  heic: FileImage,
  svg: FileImage,
  mp4: FileVideo,
  mov: FileVideo,
  mp3: FileAudio,
  wav: FileAudio,
  zip: FileArchive,
  rar: FileArchive,
  '7z': FileArchive,
  gz: FileArchive,
  doc: FileText,
  docx: FileText,
  txt: FileText,
  md: FileText,
  rtf: FileText,
  csv: FileSpreadsheet,
  xls: FileSpreadsheet,
  xlsx: FileSpreadsheet,
  ppt: Presentation,
  pptx: Presentation,
  eml: FileText,
}

function iconFor(attachment: AttachmentRow): LucideIcon {
  const mime = attachment.mime?.toLowerCase() ?? ''

  // `octet-stream` is what a sender writes when their mail client did not know either. Reading
  // it as "binary blob" and drawing a generic icon would take that guess as fact.
  if (mime !== '' && !mime.startsWith('application/octet-stream')) {
    for (const [prefix, icon] of BY_MIME) {
      if (mime.startsWith(prefix)) return icon
    }
  }

  const extension = attachment.filename?.toLowerCase().split('.').pop() ?? ''

  return BY_EXTENSION[extension] ?? File
}

/**
 * The kind of file, in words, for the screen reader.
 *
 * The icon carries this for everyone else, and "PDF document, 37 KB" is a great deal more use
 * than the filename alone when the filename is a hex string — which, for anything a bank sends,
 * it usually is.
 */
function kindOf(attachment: AttachmentRow): string {
  const extension = attachment.filename?.toLowerCase().split('.').pop() ?? ''

  return extension === '' ? 'File' : extension.toUpperCase()
}

export function AttachmentTile({ attachment, onOpen, onSave }: AttachmentTileProps) {
  const Icon = iconFor(attachment)
  const name = attachment.filename ?? 'Attachment'
  const size = formatFileSize(attachment.size ?? 0)

  return (
    <div className={styles.tile}>
      {/* The card is the button, so the whole thing is a target rather than the filename being
          a small one. The save control sits outside it — nesting a button inside a button is
          invalid, and a click on it must not also open the preview. */}
      <button
        type="button"
        className={styles.open}
        onClick={onOpen}
        aria-label={`Preview ${name}, ${kindOf(attachment)}, ${size}`}
      >
        <span className={styles.glyph} aria-hidden="true">
          <Icon className={styles.icon} strokeWidth={1.25} />
          <span className={styles.kind}>{kindOf(attachment)}</span>
        </span>

        <span className={styles.text}>
          {/* Two lines and then an ellipsis. A bank's filename is a hex string that will not
              fit on one line and is not worth three. */}
          <span className={styles.name}>{name}</span>
          <span className={styles.size}>{size}</span>
        </span>
      </button>

      <IconButton icon={Download} label={`Save ${name}`} className={styles.save} onClick={onSave} />
    </div>
  )
}
