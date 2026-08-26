import { useState } from 'react'

import { useAccountEvents, useAccountsDetail } from './queries'

export interface AccountsGateState {
  /** True once the core has answered and there are no accounts at all. */
  firstRun: boolean
  settingsOpen: boolean
  openSettings: () => void
  closeSettings: () => void
}

/**
 * The first-run check and the Settings sheet's open state.
 *
 * A hook in its own file rather than beside `AccountsGate`, so that file exports components
 * only and Fast Refresh keeps working — the same reason `lib/tint.ts` is separate from
 * `Avatar`.
 *
 * Called once, near the root: `useAccountEvents` subscribes to the core's `accounts:changed`
 * and there should be exactly one such subscription.
 */
export function useAccountsGate(): AccountsGateState {
  useAccountEvents()

  const accounts = useAccountsDetail()
  const [settingsOpen, setSettingsOpen] = useState(false)

  // `isSuccess` rather than a length check on possibly-undefined data: the assistant must
  // not flash open during the first load, before the core has answered.
  const firstRun = accounts.isSuccess && accounts.data.length === 0

  return {
    firstRun,
    settingsOpen,
    openSettings: () => {
      setSettingsOpen(true)
    },
    closeSettings: () => {
      setSettingsOpen(false)
    },
  }
}
