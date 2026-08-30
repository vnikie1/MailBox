import { useEffect, useState } from 'react'

import { remoteImagesEnabled, setRemoteImagesEnabled } from '@/lib/ipc'

import styles from '@/features/settings/settings.module.css'

/**
 * The Reading section of Settings. docs/01 §5.
 *
 * One control, and it is the one setting in this app whose default was chosen against the
 * security advice. It is here, described plainly, because a default like this should be
 * something the owner can see and change rather than something they have to discover.
 */
export function ReadingSettings() {
  const [images, setImages] = useState<boolean | null>(null)

  useEffect(() => {
    void remoteImagesEnabled().then(setImages)
  }, [])

  return (
    <section className={styles.section}>
      <h3 className={styles.heading}>Reading</h3>

      <label className={styles.choice}>
        <input
          type="checkbox"
          className={styles.checkbox}
          checked={images === true}
          disabled={images === null}
          onChange={(event) => {
            setImages(event.target.checked)
            void setRemoteImagesEnabled(event.target.checked)
          }}
        />
        Load images in messages automatically
      </label>

      <p className={styles.hint}>
        {images === true
          ? 'The sender learns you opened the message, roughly when, and the IP address you opened it from. They do not learn which app you use or which message it was, and nothing is remembered between senders. Any message can be blocked from its banner.'
          : 'Messages show a banner instead, and images load only when you ask. Nothing tells the sender you opened the message.'}
      </p>
    </section>
  )
}
