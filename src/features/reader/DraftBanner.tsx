import { FilePenLine } from 'lucide-react'

import { useMailboxes } from '@/app/queries'
import type { MessageFull } from '@/lib/generated/MessageFull'
import { composeOpenDraft, reasonFor } from '@/lib/ipc'
import { Button, useToast } from '@/ui'

// The junk banner's look: one strip above the body, a sentence and a button.
import styles from './JunkBanner.module.css'

export interface DraftBannerProps {
  message: MessageFull
}

/**
 * The way back into a saved draft. Edit Draft.
 *
 * A draft could be saved and never opened again anywhere in the app: the Drafts mailbox showed it
 * like any other message, read-only, and nothing offered to edit it (C3, 2026-11-14 audit). Shown
 * for a message in a mailbox with the Drafts role, which is where every saved draft is filed.
 */
export function DraftBanner({ message }: DraftBannerProps) {
  const { data: mailboxes = [] } = useMailboxes()
  const toast = useToast()

  const role = mailboxes.find((mailbox) => mailbox.id === message.mailboxId)?.role
  if (role !== 'drafts') return null

  return (
    <div className={styles.banner} role="status">
      <FilePenLine className={styles.icon} aria-hidden strokeWidth={1.5} />

      <p className={styles.text}>This is a draft. It has not been sent.</p>

      <Button
        variant="bordered"
        onClick={() => {
          composeOpenDraft(message.id).catch((cause: unknown) => {
            toast.show({ title: 'The draft could not be opened', description: reasonFor(cause) })
          })
        }}
      >
        Edit Draft
      </Button>
    </div>
  )
}
