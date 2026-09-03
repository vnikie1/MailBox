import { useMemo } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'

import { FirstRun, useAccountsGate } from '@/features/accounts'
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

  const { firstRun } = useAccountsGate()

  return (
    <>
      <SyncContext.Provider value={sync}>
        <AppShell />
      </SyncContext.Provider>

      {/* Bottom-centre, floating over the panes. Undo Send only means anything while the
          message is still held, so the banner has to be visible from wherever the user is. */}
      <OutboxBanner />

      <FirstRun firstRun={firstRun} />
    </>
  )
}

export function App() {
  const client = useMemo(createClient, [])

  return (
    <QueryClientProvider client={client}>
      {/* Outside `Shell` rather than inside it, so the hooks in `Shell`'s own body are within
          the provider too. It used to be rendered *by* `Shell`, which cannot cover them: a
          component's hooks run before anything it returns exists. That went unnoticed until a
          hook up there needed a toast — `useSystemEvents` calls the archive and mark-read
          mutations, and giving those an error toast made `useToast` throw during render and
          took the whole window down to a blank page. */}
      <ToastProvider>
        <Shell />
      </ToastProvider>
    </QueryClientProvider>
  )
}
