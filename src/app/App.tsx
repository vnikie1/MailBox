import { useMemo } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'

import { AccountsGate, useAccountsGate } from '@/features/accounts'
import { OutboxBanner } from '@/features/outbox/OutboxBanner'
import { AppShell } from '@/features/shell/AppShell'
import { ToastProvider } from '@/ui'

import { useAppearanceSync } from './useAppearanceSync'
import { useMailEvents } from './queries'
import { SyncContext, useSync } from './useSync'
import { useSystemEvents } from './useSystemEvents'

/**
 * The application.
 *
 * The QueryClient is configured with **no polling anywhere** — standing rule 14 makes that
 * a rule rather than a default, and `refetchInterval` is the one setting that would break
 * it silently. Freshness comes from the core's events instead, via `useMailEvents`.
 *
 * `staleTime` is deliberately long: a mailbox list that has not been announced as changed
 * has not changed, so refetching it because a window regained focus is work with no
 * possible new answer.
 */
function createClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 5 * 60 * 1000,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
        retry: 1,
      },
    },
  })
}

function Shell() {
  useAppearanceSync()
  useMailEvents()
  const sync = useSync()
  useSystemEvents()

  const { firstRun, settingsOpen, openSettings, closeSettings } = useAccountsGate()

  return (
    <ToastProvider>
      <SyncContext.Provider value={sync}>
        <AppShell onOpenSettings={openSettings} />
      </SyncContext.Provider>

      {/* Bottom-centre, floating over the panes. Undo Send only means anything while the
          message is still held, so the banner has to be visible from wherever the user is. */}
      <OutboxBanner />

      {/* Inside the provider, because the assistant reports what it did with a toast. */}
      <AccountsGate
        firstRun={firstRun}
        settingsOpen={settingsOpen}
        onCloseSettings={closeSettings}
      />
    </ToastProvider>
  )
}

export function App() {
  const client = useMemo(createClient, [])

  return (
    <QueryClientProvider client={client}>
      <Shell />
    </QueryClientProvider>
  )
}
