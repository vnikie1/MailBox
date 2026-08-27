import { FlagOff } from 'lucide-react'
import { useEffect, useState } from 'react'

import type { FlagName } from '@/lib/generated/FlagName'
import { flagNames, flagSet, onChanged } from '@/lib/organise'
import { Menu, MenuItem, MenuSeparator, useToast, type MenuProps } from '@/ui'

export interface FlagMenuProps {
  /** What the flag applies to. */
  ids: number[]
  /** Which colour these messages already carry, when they agree on one. */
  current?: string | null
  /** Required: both of these are root menus, and a root menu without a trigger cannot open. */
  trigger: NonNullable<MenuProps['trigger']>
}

/**
 * The seven flag colours. docs/01 §8.
 *
 * The swatch is the point of this menu: Mail's flag colours are chosen by eye from a row of
 * dots, not by reading seven words. The names are there for screen readers, and for the ones
 * that have been renamed, but they are not what a sighted user is looking at.
 *
 * Names come from the core rather than a constant here, because they are renameable and a
 * hard-coded "Red" would go stale the moment someone renames it to "Invoices".
 */
export function FlagMenu({ ids, current, trigger }: FlagMenuProps) {
  const [names, setNames] = useState<FlagName[]>([])
  const toast = useToast()

  useEffect(() => {
    let live = true

    const load = () => {
      void flagNames().then((loaded) => {
        if (live) setNames(loaded)
      })
    }

    load()
    const unlisten = onChanged('flags:changed', load)

    return () => {
      live = false
      void unlisten.then((stop) => {
        stop()
      })
    }
  }, [])

  const apply = (color: string | null) => {
    void flagSet(ids, color).catch((error: unknown) => {
      // Surfaced rather than swallowed. A flag that silently fails to apply is the kind of
      // thing a user only notices a week later, when the smart mailbox built on it turns out
      // to be missing half its mail.
      toast.show({
        title: 'The flag could not be set',
        description: error instanceof Error ? error.message : String(error),
      })
    })
  }

  return (
    <Menu label="Flag" trigger={trigger}>
      {names.map((flag) => (
        <MenuItem
          key={flag.color}
          label={flag.name}
          checked={current === flag.color}
          swatch={flag.color}
          onClick={() => {
            apply(flag.color)
          }}
        />
      ))}

      <MenuSeparator />

      <MenuItem
        label="Clear Flag"
        icon={FlagOff}
        disabled={current === null || current === undefined}
        onClick={() => {
          apply(null)
        }}
      />
    </Menu>
  )
}
