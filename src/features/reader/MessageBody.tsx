import { useEffect, useState } from 'react'
import { ImageOff } from 'lucide-react'

import { useMessageBody } from '@/app/queries'
import { remoteImagesEnabled } from '@/lib/ipc'
import { Button } from '@/ui'

import { MessageFrame } from './MessageFrame'

import styles from './MessageBody.module.css'

/**
 * A message body, rendered. docs/03-architecture.md §6.
 *
 * This component owns the *decision* about remote images — the setting, the per-message
 * override, and the two banners. The frame it renders into, along with the sandbox and the
 * CSP, is `MessageFrame`, shared with the `.eml` viewer so there is one security boundary
 * rather than two that can drift apart.
 */

export interface MessageBodyProps {
  messageId: number
  className?: string | undefined
}

export function MessageBody({ messageId, className }: MessageBodyProps) {
  // `null` until the setting has been read, so the body is not rendered twice — once with
  // images blocked and again a moment later with them allowed, which flashes the banner on
  // screen for every message.
  const [preference, setPreference] = useState<boolean | null>(null)
  const [override, setOverride] = useState<boolean | null>(null)

  useEffect(() => {
    void remoteImagesEnabled().then(setPreference)
  }, [])

  // The per-message override is cleared on every message. Someone who blocked one sender's
  // pictures has not asked to block the next one's, and someone who allowed one has not
  // changed their standing preference.
  useEffect(() => {
    setOverride(null)
  }, [messageId])

  const loadRemote = override ?? preference ?? false

  // A query, not an effect, so it is invalidated by `messages:updated` when the body
  // finishes downloading. Bodies are fetched lazily *after* selection, so the first render
  // of a message that has just been clicked legitimately has nothing to show — and without
  // the invalidation it would go on showing nothing until the user clicked away and back.
  // Held back until the preference is known, so the first render is the right one.
  const { data: rendered, isPending } = useMessageBody(
    preference === null ? null : messageId,
    loadRemote,
  )

  const hasContent = rendered !== undefined && rendered.html.trim() !== ''
  const empty = rendered !== undefined && !hasContent

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
              setOverride(true)
            }}
          >
            Load Images
          </Button>
        </div>
      )}

      {/* The other direction, and it only appears when images *did* load. Someone who opens a
          message from a stranger and realises what that just told them needs a way to stop it
          for the rest of the thread — and with the setting on by default, this is the only
          per-message control they have. */}
      {loadRemote && rendered.loadedRemote > 0 && (
        <div className={styles.banner} role="status">
          <ImageOff className={styles.bannerIcon} aria-hidden />
          <span className={styles.bannerText}>
            Remote images loaded, which tells the sender you opened this.
          </span>
          <Button
            variant="bordered"
            onClick={() => {
              setOverride(false)
            }}
          >
            Block Images
          </Button>
        </div>
      )}

      <MessageFrame
        html={rendered.html}
        fromPlainText={rendered.fromPlainText}
        resetKey={messageId}
      />
    </div>
  )
}
