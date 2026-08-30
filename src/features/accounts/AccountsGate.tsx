import { useEffect, useState } from 'react'

import { AccountAssistant } from './AccountAssistant'

/**
 * The first-run path.
 *
 * Mail opens its account assistant on a first launch rather than showing an empty window with
 * no explanation of what to do, and so does this. The distinction that matters is that the
 * first-run sheet has **no Cancel** — dismissing it would leave the user in an app with nothing
 * in it and no visible way forward.
 *
 * Until Phase 11 this also mounted the settings sheet, and the two had nothing to do with each
 * other beyond both involving accounts. Settings is a window of its own now (see
 * `features/settings`), which leaves this component doing the one job its name describes.
 *
 * State comes from `useAccountsGate`, which the root calls once.
 */
export interface AccountsGateProps {
  firstRun: boolean
}

export function AccountsGate({ firstRun }: AccountsGateProps) {
  const [assistantOpen, setAssistantOpen] = useState(false)

  useEffect(() => {
    if (firstRun) setAssistantOpen(true)
  }, [firstRun])

  return (
    <AccountAssistant open={assistantOpen && firstRun} firstRun onOpenChange={setAssistantOpen} />
  )
}
