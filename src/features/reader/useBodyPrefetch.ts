import { useEffect, useRef } from 'react'

import { bodiesEnsure } from '@/lib/ipc'
import type { MessageRow } from '@/lib/generated/MessageRow'

/**
 * Downloads the selected message's body, and the next three rows' bodies ahead of time.
 *
 * docs/06 Phase 5 §3 — *lazy body fetch on selection + prefetch of the next 3 rows*.
 *
 * Three, not thirty. The point of prefetching is that arrowing down a list feels instant;
 * fetching further ahead than the user can plausibly move spends their bandwidth and the
 * server's connection budget on messages they will never open. Three covers a key held down
 * for a second, which is as far as anyone reads without stopping.
 *
 * Every id already downloaded is free — the core checks `body_state` before opening a
 * connection — so this deliberately does not track what it has fetched. Duplicating that
 * bookkeeping in the UI is how the two copies drift apart.
 */
const PREFETCH_AHEAD = 3

/**
 * Downloads the bodies of every message the reader is actually showing.
 *
 * Separate from the list prefetch above, and needed because the two do not see the same
 * messages. The list prefetch works from the rows on screen; the reader shows a *thread*, and
 * a thread reaches across mailboxes — a reply that also carries a Gmail label appears in the
 * conversation while living in a different mailbox entirely, so the list never mentions it.
 *
 * Without this, such a message sat on "Downloading this message…" forever: nothing had asked
 * for it, and nothing ever would. Found by running the app against a real Gmail account,
 * where labels make it the common case rather than an edge one.
 *
 * Grouped by account because a thread can span them, and the core's fetch is per account.
 */
export function useThreadBodies(messages: { id: number; accountId: number }[]): void {
  const lastRequested = useRef('')

  useEffect(() => {
    if (messages.length === 0) return

    const key = messages.map((message) => message.id).join(',')
    if (key === lastRequested.current) return
    lastRequested.current = key

    ensureByAccount(messages)
  }, [messages])
}

/**
 * One `bodiesEnsure` per account, each with only that account's ids, in the order given.
 *
 * The core opens the account's own server for a call, so an id from another account sent in the
 * same call is looked up on the wrong server. Map iteration follows insertion order, so the
 * first message's account is asked first.
 */
function ensureByAccount(messages: { id: number; accountId: number }[]): void {
  const byAccount = new Map<number, number[]>()
  for (const message of messages) {
    const existing = byAccount.get(message.accountId)
    if (existing === undefined) byAccount.set(message.accountId, [message.id])
    else existing.push(message.id)
  }

  for (const [accountId, ids] of byAccount) {
    void bodiesEnsure(accountId, ids)
  }
}

export function useBodyPrefetch(selectedId: number | undefined, visibleRows: MessageRow[]): void {
  // The last set requested, so that re-renders from unrelated state — a flag change, a
  // window resize — do not re-issue the same fetch.
  const lastRequested = useRef('')

  useEffect(() => {
    if (selectedId === undefined) return

    // A selection that is not among the rows has no account this hook can know. It used to be
    // sent under the first row's account, which is the bug below in another form; the reader's
    // `useThreadBodies` fetches it with its real account.
    const index = visibleRows.findIndex((row) => row.id === selectedId)
    if (index < 0) return

    // The selected message first: it is the one someone is waiting for, and the core walks
    // each list in order.
    const wanted = visibleRows.slice(index, index + 1 + PREFETCH_AHEAD)

    const key = wanted.map((row) => row.id).join(',')
    if (key === lastRequested.current) return
    lastRequested.current = key

    // **Per row's own account.** This sent all four ids under the selected row's account, and
    // in All Inboxes, a smart mailbox or a search the next rows are routinely another account's.
    // The core fetched each by its UID and folder path from the wrong server and stored what it
    // found there as the body: someone else's message, shown, quoted in replies, forwarded.
    ensureByAccount(wanted)
  }, [selectedId, visibleRows])
}
