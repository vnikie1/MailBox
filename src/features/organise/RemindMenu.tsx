import { AlarmClock, BellOff, CalendarClock, Clock, Sunrise } from 'lucide-react'

import { snooze, unsnooze } from '@/lib/organise'
import { Menu, MenuItem, MenuSeparator, useToast, type MenuProps } from '@/ui'

export interface RemindMenuProps {
  ids: number[]
  /** Whether any of the selection is already snoozed, which is what enables Cancel. */
  anySnoozed?: boolean
  /**
   * The control that opens it, when this is a root menu.
   *
   * Optional, where it used to be required. A root menu without a trigger cannot open — but
   * this is also mounted *inside* the message context menu, where it is a submenu and the row
   * with the chevron is its own trigger. Passing one there would draw a button inside a menu.
   */
  trigger?: MenuProps['trigger']
  /** The leading icon, for when this is a submenu row. */
  icon?: MenuProps['icon']
}

/**
 * The choices Mail offers, in the order it offers them.
 *
 * Every one of these is computed against the **user's** clock and calendar rather than being
 * a fixed number of seconds, because "tomorrow" is not "in 24 hours" — it is a date, and on
 * the two nights a year the clocks change it is a different number of hours. Working in local
 * `Date` arithmetic gets that right for free; adding seconds to a timestamp does not.
 */
function tonight(): number {
  const when = new Date()
  when.setHours(20, 0, 0, 0)

  // Past eight already: the user means tomorrow evening, not a reminder in the past.
  if (when.getTime() <= Date.now()) when.setDate(when.getDate() + 1)

  return Math.floor(when.getTime() / 1000)
}

function tomorrowMorning(): number {
  const when = new Date()
  when.setDate(when.getDate() + 1)
  when.setHours(8, 0, 0, 0)
  return Math.floor(when.getTime() / 1000)
}

function thisWeekend(): number {
  const when = new Date()
  // Saturday. `(6 - day + 7) % 7` lands on today when it is already Saturday, so a Saturday
  // afternoon "this weekend" would be a reminder that has already passed; the `|| 7` pushes
  // it to next Saturday instead.
  const days = (6 - when.getDay() + 7) % 7 || 7
  when.setDate(when.getDate() + days)
  when.setHours(9, 0, 0, 0)
  return Math.floor(when.getTime() / 1000)
}

function nextWeek(): number {
  const when = new Date()
  const days = (1 - when.getDay() + 7) % 7 || 7
  when.setDate(when.getDate() + days)
  when.setHours(8, 0, 0, 0)
  return Math.floor(when.getTime() / 1000)
}

function inAnHour(): number {
  return Math.floor(Date.now() / 1000) + 60 * 60
}

/** Remind Me. docs/01 §8. */
export function RemindMenu({ ids, anySnoozed = false, trigger, icon }: RemindMenuProps) {
  const toast = useToast()

  const set = (at: () => number, label: string) => {
    void snooze(ids, at())
      .then(() => {
        toast.show({
          title: `Reminder set for ${label.toLowerCase()}`,
          icon: AlarmClock,
        })
      })
      .catch((error: unknown) => {
        toast.show({
          title: 'The reminder could not be set',
          description: error instanceof Error ? error.message : String(error),
        })
      })
  }

  return (
    <Menu
      label="Remind Me"
      {...(icon === undefined ? {} : { icon })}
      {...(trigger === undefined ? {} : { trigger })}
    >
      <MenuItem
        label="In an Hour"
        icon={Clock}
        onClick={() => {
          set(inAnHour, 'in an hour')
        }}
      />
      <MenuItem
        label="Tonight"
        icon={Clock}
        onClick={() => {
          set(tonight, 'tonight')
        }}
      />
      <MenuItem
        label="Tomorrow"
        icon={Sunrise}
        onClick={() => {
          set(tomorrowMorning, 'tomorrow morning')
        }}
      />
      <MenuItem
        label="This Weekend"
        icon={CalendarClock}
        onClick={() => {
          set(thisWeekend, 'the weekend')
        }}
      />
      <MenuItem
        label="Next Week"
        icon={CalendarClock}
        onClick={() => {
          set(nextWeek, 'next week')
        }}
      />

      {/* Rendered only where it can do something. A message-list selection can never hold a
          snoozed message — the list query filters them out — so in the context menu this row
          would always be present and always dead, which standing rule 18 rules out. */}
      {anySnoozed && (
        <>
          <MenuSeparator />

          <MenuItem
            label="Cancel Reminder"
            icon={BellOff}
            onClick={() => {
              void unsnooze(ids)
            }}
          />
        </>
      )}
    </Menu>
  )
}
