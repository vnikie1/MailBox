import { useEffect, useState } from 'react'

import { Field, Form } from '@/features/settings/SettingsForm'
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
      <h2 className={styles.heading}>Reading</h2>

      <Form>
        {/* The two states are described in the same terms the banners use, because this
            setting and those buttons are the same decision reached from two places. Concrete
            about what is disclosed: "improves your privacy" tells a user nothing they can
            weigh. Cut to two sentences from five — the detail that survived is what somebody
            deciding actually needs, and the rest was reassurance about what is *not*
            disclosed, which nobody was worried about until we raised it. */}
        <Field
          label="Remote images"
          hint={
            images === true
              ? 'A sender who hosts their images learns that you opened the message, roughly when, and the IP address you read it from. Any single message can be stopped from its own banner.'
              : 'Nothing is fetched until you ask, so no sender learns you opened anything. Messages with images show a banner and a Show Images button.'
          }
        >
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
        </Field>
      </Form>
    </section>
  )
}
