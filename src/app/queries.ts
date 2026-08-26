import { useEffect } from 'react'
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from '@tanstack/react-query'
import type { UnlistenFn } from '@tauri-apps/api/event'

import type { Cursor } from '@/lib/generated/Cursor'
import type { FlagPatch } from '@/lib/generated/FlagPatch'
import type { MailboxRow } from '@/lib/generated/MailboxRow'
import type { MessageFull } from '@/lib/generated/MessageFull'
import type { MessageRow } from '@/lib/generated/MessageRow'
import * as ipc from '@/lib/ipc'

/**
 * Server state, over the IPC contract. docs/03-architecture.md §4.
 *
 * The rule that shapes this file is standing rule 14: **the UI never polls.** Nothing here
 * has a refetch interval. The core pushes `mailbox:changed` and `messages:updated`, and the
 * only thing the UI does in response is invalidate the affected query keys — which is what
 * makes an account syncing in the background cost nothing while you are reading.
 *
 * Query keys are namespaced so an event can invalidate exactly what changed rather than
 * everything: `['messages', mailboxIds, …]` can be dropped without disturbing an open
 * message body under `['message', id]`.
 */

const PAGE_SIZE = 100

export const keys = {
  accounts: ['accounts'] as const,
  mailboxes: ['mailboxes'] as const,
  messages: (mailboxIds: number[], unreadOnly: boolean) =>
    ['messages', [...mailboxIds].sort((a, b) => a - b), unreadOnly] as const,
  message: (id: number) => ['message', id] as const,
  thread: (threadId: number) => ['thread', threadId] as const,
  search: (text: string, mailboxIds: number[]) => ['search', text, mailboxIds] as const,
}

export function useAccounts() {
  return useQuery({ queryKey: keys.accounts, queryFn: ipc.accountsList })
}

export function useMailboxes() {
  return useQuery<MailboxRow[]>({
    queryKey: keys.mailboxes,
    queryFn: () => ipc.mailboxesTree(),
  })
}

/**
 * The message list, paged by cursor.
 *
 * `useInfiniteQuery` rather than a plain query because the list is virtualised over a
 * mailbox that may hold a hundred thousand rows — the first page paints, and the next is
 * fetched when the virtualiser approaches the end. The cursor comes straight from the
 * previous page, so this never asks the database to count or skip anything.
 */
export function useMessages(mailboxIds: number[], unreadOnly: boolean) {
  return useInfiniteQuery({
    queryKey: keys.messages(mailboxIds, unreadOnly),
    enabled: mailboxIds.length > 0,
    initialPageParam: null as Cursor | null,
    queryFn: ({ pageParam }) =>
      ipc.messagesPage({
        mailboxIds,
        cursor: pageParam,
        limit: PAGE_SIZE,
        unreadOnly,
      }),
    getNextPageParam: (lastPage) => lastPage.nextCursor,
  })
}

export function useThread(threadId: number | null) {
  return useQuery<MessageFull[]>({
    queryKey: keys.thread(threadId ?? -1),
    enabled: threadId !== null,
    queryFn: () => ipc.threadGet(threadId ?? -1),
  })
}

export function useSearch(text: string, mailboxIds: number[]) {
  const trimmed = text.trim()

  return useQuery<MessageRow[]>({
    queryKey: keys.search(trimmed, mailboxIds),
    enabled: trimmed.length > 0,
    queryFn: () => ipc.searchMessages({ text: trimmed, mailboxIds, limit: 100 }),
  })
}

/**
 * Invalidates everything a mutation could have moved.
 *
 * Deliberately coarse on the list and precise on the rest. A flag change can reorder a
 * filtered list and move a row between mailboxes, so the list has to be refetched; an open
 * message body has not changed and refetching it would flicker the reader.
 */
function invalidateAfterMutation(client: QueryClient): void {
  void client.invalidateQueries({ queryKey: ['messages'] })
  void client.invalidateQueries({ queryKey: keys.mailboxes })
  void client.invalidateQueries({ queryKey: ['search'] })
}

export function useSetFlags() {
  const client = useQueryClient()

  return useMutation({
    mutationFn: ({ ids, patch }: { ids: number[]; patch: FlagPatch }) =>
      ipc.msgSetFlags(ids, patch),
    onSuccess: (_result, variables) => {
      // The browser has no core to push events, so mutations there announce themselves
      // through the same channel. In Tauri this is a no-op and the real event arrives.
      ipc.notifyBrowserMailboxChange([])
      invalidateAfterMutation(client)
      for (const id of variables.ids) {
        void client.invalidateQueries({ queryKey: keys.message(id) })
      }
    },
  })
}

export function useMoveMessages() {
  const client = useQueryClient()

  return useMutation({
    mutationFn: ({ ids, mailboxId }: { ids: number[]; mailboxId: number }) =>
      ipc.msgMove(ids, mailboxId),
    onSuccess: () => {
      invalidateAfterMutation(client)
    },
  })
}

export function useDeleteMessages() {
  const client = useQueryClient()

  return useMutation({
    mutationFn: ({ ids, permanent }: { ids: number[]; permanent: boolean }) =>
      ipc.msgDelete(ids, permanent),
    onSuccess: () => {
      invalidateAfterMutation(client)
    },
  })
}

/**
 * Subscribes to the core's events for the life of the app.
 *
 * This is the whole of standing rule 14's implementation on the UI side: the core says what
 * changed, and the only response is to mark the matching keys stale. Nothing here fetches
 * on a timer.
 */
export function useMailEvents(): void {
  const client = useQueryClient()

  useEffect(() => {
    let cancelled = false
    const unlisteners: UnlistenFn[] = []

    const keep = (unlisten: UnlistenFn) => {
      if (cancelled) unlisten()
      else unlisteners.push(unlisten)
    }

    void ipc
      .onMailboxChanged(() => {
        // The event carries the new counts, but the sidebar reads them from the mailbox
        // query — invalidating is one line and cannot drift from what the store holds.
        void client.invalidateQueries({ queryKey: keys.mailboxes })
      })
      .then(keep)

    void ipc
      .onMessagesUpdated((ids) => {
        void client.invalidateQueries({ queryKey: ['messages'] })
        for (const id of ids) {
          void client.invalidateQueries({ queryKey: keys.message(id) })
        }

        // The rendered body too. A message is selected before its body has downloaded —
        // that is the whole point of fetching lazily — so the reader renders an empty
        // frame first and needs telling when the content actually arrives. Without this
        // the body appeared only if you clicked away and back.
        void client.invalidateQueries({ queryKey: ['messageBody'] })
      })
      .then(keep)

    return () => {
      cancelled = true
      unlisteners.forEach((unlisten) => {
        unlisten()
      })
    }
  }, [client])
}

/**
 * A message's rendered body. docs/03 §6.
 *
 * A query rather than an effect so that it participates in the same invalidation the rest of
 * the app uses: the body arrives *after* the message is selected, and `messages:updated` is
 * what says so.
 *
 * `loadRemote` is part of the key, so consenting to remote images is a different question
 * with a different answer rather than a mutation of the current one.
 */
export function useMessageBody(messageId: number | null, loadRemote: boolean) {
  return useQuery({
    queryKey: ['messageBody', messageId, loadRemote] as const,
    queryFn: () => ipc.messageBody(messageId ?? 0, loadRemote),
    enabled: messageId !== null,
    // Bodies are immutable once downloaded; the only thing that changes is whether we have
    // one yet, and the event above covers that.
    staleTime: Infinity,
  })
}
