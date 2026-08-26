import { useCallback } from 'react'
import { useQueryClient } from '@tanstack/react-query'

import { notifyBrowserAccountsChanged } from '@/lib/ipc'

import { accountKeys } from './queries'

/**
 * Call after any mutation that changes accounts or the OAuth client configuration.
 *
 * Two things, deliberately, and the reason is a bug that shipped:
 *
 * * **It invalidates the queries directly.** `useProviders` is cached with
 *   `staleTime: Infinity`, so a provider list that has been read once is never refetched on
 *   its own. Saving a Google client id therefore left the provider tile greyed out — with
 *   the client id sitting correctly in the database the whole time — until the app was
 *   restarted. Relying on the core to announce a change means every command has to remember
 *   to announce it, and `oauth_client_set` did not.
 * * **It still pokes the browser bus.** `notifyBrowserAccountsChanged` is a no-op inside
 *   Tauri and the only notification path when served by Vite, so the browser store's
 *   listeners keep working.
 *
 * The core's `accounts:changed` event is not redundant: it is what keeps a *second* window,
 * or a change made by the sync engine, in step. This is the local half.
 */
export function useAccountsChanged(): () => void {
  const client = useQueryClient()

  return useCallback(() => {
    notifyBrowserAccountsChanged()

    void client.invalidateQueries({ queryKey: accountKeys.providers })
    void client.invalidateQueries({ queryKey: ['accounts'] })
    void client.invalidateQueries({ queryKey: ['mailboxes'] })
    void client.invalidateQueries({ queryKey: ['messages'] })
  }, [client])
}
