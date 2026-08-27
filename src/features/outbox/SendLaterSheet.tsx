import { useState } from 'react'

import { Button, Sheet, TextField } from '@/ui'

export interface SendLaterSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Called with the chosen moment, in seconds since the epoch. */
  onChoose: (sendAfter: number) => void
}

/**
 * "Custom…" for Send Later. docs/06 Phase 7.
 *
 * Two plain inputs rather than a calendar. A date picker is a lot of interface for a decision
 * that is nearly always "tomorrow, some time in the morning", and the native controls already
 * follow the user's own locale and first-day-of-week — which a hand-built calendar has to be
 * told, and gets wrong for exactly the people whose conventions differ from the author's.
 */
export function SendLaterSheet({ open, onOpenChange, onChoose }: SendLaterSheetProps) {
  const [date, setDate] = useState(() => defaultDate())
  const [time, setTime] = useState('08:00')
  const [problem, setProblem] = useState<string | null>(null)

  const confirm = () => {
    // Parsed as *local* time on purpose. `new Date('2026-08-28')` is midnight UTC and would
    // schedule a message for the previous evening in every timezone west of London.
    const when = new Date(`${date}T${time}`)

    if (Number.isNaN(when.getTime())) {
      setProblem('That is not a date and time.')
      return
    }

    if (when.getTime() <= Date.now()) {
      // Refused rather than sent immediately. Someone who picked a past time meant a future
      // one and mistyped it, and sending straight away is the one outcome they cannot undo.
      setProblem('That moment has already passed.')
      return
    }

    setProblem(null)
    onChoose(Math.floor(when.getTime() / 1000))
    onOpenChange(false)
  }

  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title="Send Later"
      description="The message stays in the outbox until then."
      footer={
        <>
          <Button
            variant="bordered"
            onClick={() => {
              onOpenChange(false)
            }}
          >
            Cancel
          </Button>
          <Button variant="filled" onClick={confirm}>
            Schedule
          </Button>
        </>
      }
    >
      <TextField
        label="Date"
        type="date"
        value={date}
        onChange={(event) => {
          setDate(event.target.value)
          setProblem(null)
        }}
      />
      <TextField
        label="Time"
        type="time"
        value={time}
        onChange={(event) => {
          setTime(event.target.value)
          setProblem(null)
        }}
        {...(problem === null ? {} : { invalid: true, hint: problem })}
      />
    </Sheet>
  )
}

/** Tomorrow, in the `yyyy-mm-dd` an `<input type="date">` expects. */
function defaultDate(): string {
  const when = new Date()
  when.setDate(when.getDate() + 1)

  // Built from the local parts rather than `toISOString`, which converts to UTC first and so
  // returns yesterday's date for anyone east of Greenwich late in the evening.
  const year = when.getFullYear()
  const month = String(when.getMonth() + 1).padStart(2, '0')
  const day = String(when.getDate()).padStart(2, '0')

  return `${year}-${month}-${day}`
}
