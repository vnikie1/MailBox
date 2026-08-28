import { GROUP_ORDER, SHORTCUTS } from '@/app/shortcuts'
import { Sheet } from '@/ui'

import styles from './ShortcutSheet.module.css'

export interface ShortcutSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

/**
 * The keyboard reference. docs/06 Phase 10.
 *
 * Rendered from `SHORTCUTS` — the same array the dispatcher binds — rather than from a list
 * written out here. A hand-written reference sheet drifts from the code within a week, and the
 * failure is silent in the worst direction: it documents a shortcut that no longer works, so
 * the user concludes the app is broken rather than the page.
 *
 * Chords are split into individual keys so each renders as its own cap. A single string in one
 * box reads as one enormous key, which is exactly not what `Ctrl+Shift+M` is.
 */
export function ShortcutSheet({ open, onOpenChange }: ShortcutSheetProps) {
  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title="Keyboard Shortcuts"
      className={styles.sheet}
    >
      <div className={styles.groups}>
        {GROUP_ORDER.map((group) => {
          const rows = SHORTCUTS.filter((shortcut) => shortcut.group === group)
          if (rows.length === 0) return null

          return (
            <section key={group} className={styles.group}>
              <h3 className={styles.heading}>{group}</h3>

              <dl className={styles.list}>
                {rows.map((shortcut) => (
                  <div key={shortcut.id} className={styles.row}>
                    <dt className={styles.label}>{shortcut.label}</dt>
                    <dd className={styles.keys}>
                      {shortcut.keys.split('+').map((key, index) => (
                        // Positional by nature — the parts of one chord, in order.
                        <kbd key={`${shortcut.id}-${String(index)}`} className={styles.key}>
                          {key.trim()}
                        </kbd>
                      ))}
                    </dd>
                  </div>
                ))}
              </dl>
            </section>
          )
        })}
      </div>
    </Sheet>
  )
}
