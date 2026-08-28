import type { LucideIcon } from 'lucide-react'
import type { ReactNode } from 'react'

import styles from './EmptyState.module.css'

export interface EmptyStateProps {
  icon?: LucideIcon
  title: string
  /** One sentence. Anything longer is not read in a pane somebody expected mail in. */
  description?: string
  /** A way out. See the note on dead ends. */
  action?: ReactNode
  /**
   * `polite` for an ordinary empty pane, `assertive` for something that went wrong.
   *
   * A screen reader user gets no other signal that a pane changed from a list of forty
   * messages to a sentence — the visual difference is the whole message, and without a live
   * region it is silent.
   */
  tone?: 'neutral' | 'error'
  /**
   * `hero` is the full-pane "No Message Selected" of docs/02 §6.10: a large tertiary title and
   * nothing else. It exists because that state is not an absence to explain — it is the reader
   * at rest, and the ordinary treatment (dark title, sentence beneath, a button) would make a
   * problem out of the app simply waiting for you to pick something.
   */
  variant?: 'default' | 'hero'
  /**
   * For the background and sizing a host pane owns. Not for restyling the contents.
   *
   * Explicitly `| undefined` because `exactOptionalPropertyTypes` is on and a CSS-module class
   * is typed as possibly absent, so `className?: string` would reject every real caller.
   */
  className?: string | undefined
}

/**
 * The one component for every "there is nothing here". docs/06 Phase 10.
 *
 * The gate for this phase is *no dead ends*, and a dead end is not an ugly screen — it is a
 * screen that tells you something is absent without telling you what to do about it. An empty
 * pane with no explanation is indistinguishable from a broken one, and the user's next move is
 * to reload, or to conclude the app does not work.
 *
 * So every state gets three things: what is true, why, and — where one exists — a way out. The
 * `action` is optional because sometimes there genuinely is nothing to do; "no messages match
 * that search" needs no button, and inventing one would be worse than the silence.
 */
export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
  tone = 'neutral',
  variant = 'default',
  className,
}: EmptyStateProps) {
  return (
    <div
      className={className === undefined ? styles.wrap : `${styles.wrap} ${className}`}
      data-tone={tone}
      data-variant={variant}
      // An error is worth interrupting for; an empty mailbox is not.
      role={tone === 'error' ? 'alert' : 'status'}
      aria-live={tone === 'error' ? 'assertive' : 'polite'}
    >
      {Icon && <Icon className={styles.glyph} aria-hidden="true" strokeWidth={1.25} />}

      <p className={styles.title}>{title}</p>
      {description !== undefined && <p className={styles.description}>{description}</p>}
      {action !== undefined && <div className={styles.action}>{action}</div>}
    </div>
  )
}
