import { useEffect } from 'react'
import { useQuery, useQueryClient, type UseQueryResult } from '@tanstack/react-query'

import type { AccountDetail } from '@/lib/generated/AccountDetail'
import type { OAuthClientStatus } from '@/lib/generated/OAuthClientStatus'
import type { ProviderInfo } from '@/lib/generated/ProviderInfo'
import { accountsDetail, oauthClientGet, onAccountsChanged, providersList } from '@/lib/ipc'

/**
 * Account queries.
 *
 * Freshness comes from the core's `accounts:changed` event, never from a poll — standing
 * rule 14. `useAccountEvents` is mounted once, near the root, and invalidates the keys
 * below when the core says something moved.
 */

export const accountKeys = {
  providers: ['providers'] as const,
  detail: ['accounts', 'detail'] as const,
  oauthClient: (provider: string) => ['accounts', 'oauthClient', provider] as const,
}

export function useProviders(): UseQueryResult<ProviderInfo[]> {
  return useQuery({
    queryKey: accountKeys.providers,
    queryFn: providersList,
    // The provider table is a constant on the Rust side except for whether an OAuth client
    // is configured, which arrives as an invalidation rather than a refetch interval.
    staleTime: Infinity,
  })
}

export function useAccountsDetail(): UseQueryResult<AccountDetail[]> {
  return useQuery({
    queryKey: accountKeys.detail,
    queryFn: accountsDetail,
  })
}

export function useOAuthClient(provider: string): UseQueryResult<OAuthClientStatus> {
  return useQuery({
    queryKey: accountKeys.oauthClient(provider),
    queryFn: () => oauthClientGet(provider),
  })
}

export function useAccountEvents(): void {
  const client = useQueryClient()

  useEffect(() => {
    let unlisten: (() => void) | undefined
    let cancelled = false

    void onAccountsChanged(() => {
      // The mailbox tree and the message list both hang off accounts, so an account
      // appearing or being removed invalidates more than the accounts pane.
      void client.invalidateQueries({ queryKey: accountKeys.detail })
      void client.invalidateQueries({ queryKey: accountKeys.providers })
      void client.invalidateQueries({ queryKey: ['accounts'] })
      void client.invalidateQueries({ queryKey: ['mailboxes'] })
      void client.invalidateQueries({ queryKey: ['messages'] })
    }).then((off) => {
      if (cancelled) off()
      else unlisten = off
    })

    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [client])
}
