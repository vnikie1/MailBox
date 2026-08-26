import { useEffect, useRef, type ComponentPropsWithoutRef, type UIEvent } from 'react'

import { cx } from '@/lib/cx'
import { durationToken } from '@/lib/tokens'

import styles from './ScrollArea.module.css'

const FADE_DELAY_FALLBACK_MS = 800

export interface ScrollAreaProps extends ComponentPropsWithoutRef<'div'> {
  orientation?: 'vertical' | 'horizontal' | 'both'
}

/**
 * A scrolling surface with macOS-style overlay scrollbars. docs/01 §9.8.
 *
 * The thumb is invisible at rest and fades in while you scroll or hover, which is what
 * macOS does and what Windows' own scrollbars do not. That behaviour needs script: the
 * `::-webkit-scrollbar` pseudo-elements have no "is scrolling" state, so a timer marks
 * the element and CSS keys off the mark.
 *
 * Two things worth knowing before reusing this:
 *
 *  - The gutter is reserved permanently. Chromium lays out a styled `::-webkit-scrollbar`
 *    as a classic scrollbar, so the alternative is content reflowing by 15px the moment a
 *    list becomes long enough to scroll — which standing rule 6 forbids. Phase 2 checks
 *    the resulting text inset against `assets/reference/`.
 *  - `scrollbar-width` is deliberately not set anywhere. Chromium ignores every
 *    `::-webkit-scrollbar` rule on an element that specifies the standard property, so
 *    adding it "for Firefox" would silently delete this entire design.
 */
export function ScrollArea({
  orientation = 'vertical',
  className,
  onScroll,
  children,
  ...rest
}: ScrollAreaProps) {
  const timer = useRef(0)

  useEffect(
    () => () => {
      window.clearTimeout(timer.current)
    },
    [],
  )

  const handleScroll = (event: UIEvent<HTMLDivElement>) => {
    const element = event.currentTarget
    element.dataset.scrolling = ''

    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(
      () => {
        delete element.dataset.scrolling
      },
      durationToken('--scrollbar-fade-delay', FADE_DELAY_FALLBACK_MS, element),
    )

    onScroll?.(event)
  }

  return (
    <div
      {...rest}
      className={cx(styles.scrollArea, styles[orientation], className)}
      onScroll={handleScroll}
    >
      {children}
    </div>
  )
}
