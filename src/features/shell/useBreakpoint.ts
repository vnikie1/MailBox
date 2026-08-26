import { useEffect, useState } from 'react'

/**
 * Which layout the window is wide enough for. docs/01 §1.
 *
 *   three   >= 1000    sidebar + list + reader
 *   two     >= 700     list + reader, sidebar available as an overlay
 *   one     <  700     one pane at a time, push navigation
 *
 * Measured from the window rather than expressed as CSS media queries because the panes
 * are JavaScript-resizable anyway — the divider drag needs pixel widths, and having the
 * breakpoint live somewhere else would let the two disagree about what "1000" means.
 *
 * `docs/01` §1 calls these out as the thing "every Windows client falls apart" at, so they
 * are load-bearing rather than a nicety.
 */
export type Breakpoint = 'one' | 'two' | 'three'

export const TWO_PANE_MAX = 1000
export const ONE_PANE_MAX = 700

function breakpointFor(width: number): Breakpoint {
  if (width < ONE_PANE_MAX) return 'one'
  if (width < TWO_PANE_MAX) return 'two'
  return 'three'
}

export function useBreakpoint(): Breakpoint {
  const [breakpoint, setBreakpoint] = useState<Breakpoint>(() =>
    breakpointFor(typeof window === 'undefined' ? TWO_PANE_MAX : window.innerWidth),
  )

  useEffect(() => {
    const onResize = () => {
      // Only sets state when the bucket actually changes, so dragging a window edge does
      // not re-render three panes on every animation frame.
      setBreakpoint((current) => {
        const next = breakpointFor(window.innerWidth)
        return next === current ? current : next
      })
    }

    onResize()
    window.addEventListener('resize', onResize)
    return () => {
      window.removeEventListener('resize', onResize)
    }
  }, [])

  return breakpoint
}
