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
        Show images in messages automatically
      </label>

      {/* The two states are described in the same terms the banners use, because this setting
          and those buttons are the same decision reached from two places. Concrete about what
          is disclosed and what is not: "improves your privacy" tells a user nothing they can
          weigh, and the honest answer here is short enough to just say. */}
      <p className={styles.hint}>
        {images === true
          ? 'Most marketing email keeps its images on the sender’s own server, so showing them tells that sender you opened the message, roughly when, and the IP address you read it from. They do not learn which app you use, and nothing is shared between senders. Any message can be stopped from its own banner.'
          : 'Nothing is fetched until you ask. Messages with images show a banner and a Show Images button, so no sender learns you opened anything. Some email will look plainer until you show them.'}
      </p>
    </section>
  )
}
