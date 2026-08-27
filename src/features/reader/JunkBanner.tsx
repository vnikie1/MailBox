import { ShieldAlert } from 'lucide-react'

import type { MessageFull } from '@/lib/generated/MessageFull'
import { junkMark } from '@/lib/organise'
import { useQueryClient } from '@tanstack/react-query'

import { Button, useToast } from '@/ui'

import styles from './JunkBanner.module.css'

export interface JunkBannerProps {
  message: MessageFull
}

/**
 * The junk banner. docs/01 §8, docs/06 Phase 8.
 *
 * Two different sentences depending on who decided, and the difference is the whole design:
 * the filter's guess invites a correction, while the user's own decision is stated back to
 * them without argument. A banner that debated someone's own judgement would be the fastest
 * way to make them turn the filter off for good.
 *
 * Marking here is also the *only* way the classifier learns anything — it trains on labels a
 * human applied and on nothing else. Every press of these buttons is a training example, which
 * is why the banner appears on the message rather than as a toolbar button somewhere.
 */
export function JunkBanner({ message }: JunkBannerProps) {
  const toast = useToast()
  const client = useQueryClient()

  if (!message.isJunk) return null

  const mark = (isJunk: boolean) => {
    void junkMark([message.id], isJunk)
      .then(() => {
        // Invalidated here rather than left to the core's `mailbox:changed` event, which the
        // shell uses to refresh the *list*. The reader reads a different query, and a banner
        // that stayed on screen after "Not Junk" would look like the button did nothing.
        void client.invalidateQueries({ queryKey: ['thread'] })
        void client.invalidateQueries({ queryKey: ['message'] })
      })
      .catch((error: unknown) => {
        toast.show({
          title: 'That could not be changed',
          description: error instanceof Error ? error.message : String(error),
        })
      })
  }

  return (
    <div className={styles.banner} role="status">
      <ShieldAlert className={styles.icon} aria-hidden strokeWidth={1.5} />

      <p className={styles.text}>
        {message.junkByUser ? 'You marked this message as junk.' : 'This message looks like junk.'}
        {/* The score, but only when the filter is the one making the claim. Attaching a
            confidence to the user's own decision would be nonsense. */}
        {!message.junkByUser && message.junkScore !== null && (
          <span className={styles.score}> ({Math.round(message.junkScore * 100)}% confident)</span>
        )}
      </p>

      <Button
        variant="bordered"
        onClick={() => {
          mark(false)
        }}
      >
        Not Junk
      </Button>
    </div>
  )
}
