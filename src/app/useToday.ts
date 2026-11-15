import { useEffect, useState } from 'react'

import { storeNow } from '@/lib/ipc'

/**
 * "Now", for labelling dates — moved on at each local midnight.
 *
 * The list, the reader and the selection deck each took `now` once, when they mounted, and kept
 * it. A window left open overnight then filed yesterday's mail under Today and dated it with a
 * bare time, and the reader said "Today" about a message from the day before (M37, 2026-11-14
 * audit). Labels are by day, so a new value once a day is all they need.
 *
 * Checked again when the window becomes visible, because a timer does not run while the PC
 * sleeps: one set for midnight fires whenever the machine wakes, and a window woken on another
 * day should not wait for it.
 *
 * Outside Tauri, `storeNow` is the fixed date the browser fixtures are written against, so the
 * value never changes there and the end-to-end tests stay deterministic.
 */
export function useToday(): Date {
  const [now, setNow] = useState(storeNow)

  useEffect(() => {
    let timer: number | undefined

    const refresh = () => {
      const next = storeNow()
      setNow((current) => (sameDay(current, next) ? current : next))
    }

    const schedule = () => {
      const current = new Date()
      // A second past midnight, so the timer cannot fire a moment early and read the old day.
      const midnight = new Date(current)
      midnight.setHours(24, 0, 1, 0)

      timer = window.setTimeout(() => {
        refresh()
        schedule()
      }, midnight.getTime() - current.getTime())
    }

    const onVisible = () => {
      if (document.visibilityState === 'visible') refresh()
    }

    schedule()
    document.addEventListener('visibilitychange', onVisible)

    return () => {
      window.clearTimeout(timer)
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [])

  return now
}

function sameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  )
}
