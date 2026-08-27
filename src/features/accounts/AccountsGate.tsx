import { useEffect, useState } from 'react'

import { Sheet } from '@/ui'

import { AccountAssistant } from './AccountAssistant'
import { ComposingSettings } from '@/features/compose/ComposingSettings'

import { AccountsSettings } from './AccountsSettings'
import styles from './AccountsGate.module.css'

/**
 * The first-run path, and the way into account settings.
 *
 * Mail opens its account assistant on a first launch rather than showing an empty window
 * with no explanation of what to do, and so does this. The distinction that matters is that
 * the first-run sheet has **no Cancel** — dismissing it would leave the user in an app with
 * nothing in it and no visible way forward.
 *
 * State comes from `useAccountsGate`, which the root calls once.
 */
export interface AccountsGateProps {
  firstRun: boolean
  settingsOpen: boolean
  onCloseSettings: () => void
}

export function AccountsGate({ firstRun, settingsOpen, onCloseSettings }: AccountsGateProps) {
  const [assistantOpen, setAssistantOpen] = useState(false)

  useEffect(() => {
    if (firstRun) setAssistantOpen(true)
  }, [firstRun])

  return (
    <>
      <AccountAssistant open={assistantOpen && firstRun} firstRun onOpenChange={setAssistantOpen} />

      <Sheet
        open={settingsOpen}
        onOpenChange={(open) => {
          if (!open) onCloseSettings()
        }}
        title="Settings"
        className={styles.settingsSheet}
      >
        <AccountsSettings />
        <ComposingSettings />
      </Sheet>
    </>
  )
}
