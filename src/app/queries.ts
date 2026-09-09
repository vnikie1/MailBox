import { useCallback, useEffect } from 'react'
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
import type { Rendered } from '@/lib/generated/Rendered'
import type { Predicate } from '@/lib/generated/Predicate'
import type { FlagName } from '@/lib/generated/FlagName'
import type { SmartMailbox } from '@/lib/generated/SmartMailbox'
import type { Vip } from '@/lib/generated/Vip'
import { blockedList, flagNames, smartList, smartMessages, vipsList } from '@/lib/organise'
import { useToast } from '@/ui'
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
  // Keyed on the predicate itself, so editing a smart mailbox refetches and two smart
  // mailboxes with the same conditions share one cache entry.
  smart: (predicate: unknown) => ['messages', 'smart', predicate] as const,
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

/**
 * The messages a saved search matches. docs/01 §8.
 *
 * Offset paging rather than the keyset cursor the folder list uses. A smart mailbox is an
 * arbitrary predicate over an arbitrary set of mailboxes, so there is no single
 * `(date_received, id)` ordering the server side can seek into cheaply — and the result sets
 * are small enough that it does not matter. The folder list keeps its cursor precisely because
 * it is the one that has to page through fifty thousand rows.
 */
export function useSmartMessages(predicate: Predicate | undefined) {
  return useInfiniteQuery({
    queryKey: keys.smart(predicate ?? null),
    enabled: predicate !== undefined,
    initialPageParam: 0,
    queryFn: ({ pageParam }) =>
      predicate === undefined
        ? Promise.resolve([])
        : smartMessages(predicate, PAGE_SIZE, pageParam),
    getNextPageParam: (lastPage: MessageRow[], allPages: MessageRow[][]) =>
      lastPage.length < PAGE_SIZE ? undefined : allPages.length * PAGE_SIZE,
  })
}

/**
 * The saved searches in the sidebar, and the flag names under Flagged.
 *
 * Their own queries rather than part of `useMailboxes`, because they change for entirely
 * different reasons: a mailbox list changes when the server's folders do, and these change when
 * the user edits them. Invalidating one should not refetch the other.
 */
export function useSmartMailboxes() {
  return useQuery<SmartMailbox[]>({
    queryKey: ['smartMailboxes'],
    queryFn: smartList,
  })
}

export function useVips() {
  return useQuery<Vip[]>({
    queryKey: ['vips'],
    queryFn: vipsList,
  })
}

/**
 * The addresses the user has blocked, lowercased for comparison.
 *
 * A Set rather than the array the core returns, because the only question ever asked of it is
 * whether one sender is in it — and it is asked once per context menu, on every right-click.
 *
 * This is what lets Block Sender be a two-way switch. Without it the menu could only ever say
 * "Block", including for someone already blocked, and clicking it twice would do the same
 * thing both times with nothing to show for it.
 */
export function useBlockedSenders() {
  return useQuery<ReadonlySet<string>>({
    queryKey: ['blockedSenders'],
    queryFn: async () => new Set((await blockedList()).map((address) => address.toLowerCase())),
  })
}

export function useFlagNames() {
  return useQuery<FlagName[]>({
    queryKey: ['flagNames'],
    queryFn: flagNames,
  })
}

/** The conversation containing `messageId`. Keyed by the message, which is what the reader has. */
export function useThread(messageId: number | null) {
  return useQuery<MessageFull[]>({
    queryKey: keys.thread(messageId ?? -1),
    enabled: messageId !== null,
    queryFn: () => ipc.threadGet(messageId ?? -1),
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
  // The reader reads a *thread*, and deleting or moving the open message changes one. Without
  // this the row left the list while the reader went on rendering the message in full --
  // headers, body, attachments -- as though nothing had happened.
  void client.invalidateQueries({ queryKey: ['thread'] })
}

/**
 * Reports a mutation that did not happen.
 *
 * Every mutation below had an `onSuccess` and nothing else, so a rejected command was caught
 * by TanStack Query, stored in a state nothing read, and vanished. Pressing Delete on a
 * selection the core refuses left the messages exactly where they were, with no error anywhere
 * -- the failure this app keeps producing, and the one hardest to trust your own eyes about.
 */
function useMutationProblem(): (cause: unknown) => void {
  const toast = useToast()

  return useCallback(
    (cause: unknown) => {
      toast.show({
        title: 'That change could not be made',
        description: ipc.reasonFor(cause),
      })
    },
    [toast],
  )
}

export function useSetFlags() {
  const client = useQueryClient()
  const problem = useMutationProblem()

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
    onError: problem,
  })
}

export function useToggleRead() {
  const client = useQueryClient()
  const problem = useMutationProblem()

  return useMutation({
    mutationFn: (ids: number[]) => ipc.msgToggleRead(ids),
    onSuccess: (_result, ids) => {
      ipc.notifyBrowserMailboxChange([])
      invalidateAfterMutation(client)
      for (const id of ids) {
        void client.invalidateQueries({ queryKey: keys.message(id) })
      }
      // The reader reads a thread, not a message, so that key has to go too or the row
      // changes in the list while the open message stays bold.
      void client.invalidateQueries({ queryKey: ['thread'] })
    },
    onError: problem,
  })
}

export function useToggleFlag() {
  const client = useQueryClient()
  const problem = useMutationProblem()

  return useMutation({
    mutationFn: (ids: number[]) => ipc.msgToggleFlag(ids),
    onSuccess: (_result, ids) => {
      ipc.notifyBrowserMailboxChange([])
      invalidateAfterMutation(client)
      for (const id of ids) {
        void client.invalidateQueries({ queryKey: keys.message(id) })
      }
      void client.invalidateQueries({ queryKey: ['thread'] })
    },
    onError: problem,
  })
}

export function useArchiveMessages() {
  const client = useQueryClient()
  const problem = useMutationProblem()

  return useMutation({
    mutationFn: (ids: number[]) => ipc.msgArchive(ids),
    onSuccess: () => {
      ipc.notifyBrowserMailboxChange([])
      invalidateAfterMutation(client)
    },
    onError: problem,
  })
}

export function useMoveMessages() {
  const client = useQueryClient()
  const problem = useMutationProblem()

  return useMutation({
    mutationFn: ({ ids, mailboxId }: { ids: number[]; mailboxId: number }) =>
      ipc.msgMove(ids, mailboxId),
    onSuccess: () => {
      invalidateAfterMutation(client)
    },
    onError: problem,
  })
}

export function useDeleteMessages() {
  const client = useQueryClient()
  const problem = useMutationProblem()

  return useMutation({
    mutationFn: ({ ids, permanent }: { ids: number[]; permanent: boolean }) =>
      ipc.msgDelete(ids, permanent),
    onSuccess: () => {
      invalidateAfterMutation(client)
    },
    onError: problem,
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

        // The list and search too, because this is the only event three commands emit.
        // Undo, Mark as Junk and Run Rules do not go through a mutation hook — they call the
        // core directly — so nothing else marked the list stale. An undone move left the
        // message sitting in the folder it had been moved out of, and a message marked as
        // junk stayed in the Inbox, both until something unrelated forced a refetch. The
        // action had worked; only the screen disagreed, which is the version of this bug
        // that is hardest to trust your own eyes about.
        //
        // Coarse on purpose. A command that announces a mailbox changed cannot say which
        // rows moved, and the alternative — remembering to invalidate at every call site —
        // is exactly the bookkeeping those three call sites already forgot.
        void client.invalidateQueries({ queryKey: ['messages'] })
        void client.invalidateQueries({ queryKey: ['search'] })
        void client.invalidateQueries({ queryKey: ['thread'] })
      })
      .then(keep)

    void ipc
      .onMessagesUpdated((ids) => {
        void client.invalidateQueries({ queryKey: ['messages'] })
        for (const id of ids) {
          void client.invalidateQueries({ queryKey: keys.message(id) })
        }

        // The open conversation, which is where attachments live.
        //
        // Reported from using the app: "a mail with attachment doesn't load the attachment on
        // the first load of the mail. I have to go to other mails first and do a back and forth
        // and then after some time it loads."
        //
        // A message's attachments do not exist until its body has been downloaded and parsed —
        // `sync::bodies::persist` writes the body and the attachment rows in one transaction,
        // and until then the row is real but its attachment list is empty. Bodies are fetched
        // lazily, so a message opened the moment it arrives is routinely read *before* that
        // happens. The reader takes its attachments from `MessageFull` on this query, and
        // nothing here invalidated it — so the tile never appeared.
        //
        // "After some time" was the giveaway: `mailbox:changed` above *does* invalidate
        // `['thread']`, so the next sync tick fixed it by accident. That is what made this look
        // intermittent rather than deterministic.
        //
        // Matched on the cached rows rather than the key. The key is the *selected* message id,
        // while the data holds every message in the conversation — so a body arriving for the
        // second message of an open thread has an id that appears nowhere in the key.
        const changed = new Set(ids)
        void client.invalidateQueries({
          queryKey: ['thread'],
          predicate: (query) => {
            const thread = query.state.data as MessageFull[] | undefined
            return thread?.some((message) => changed.has(message.id)) ?? false
          },
        })

        // The rendered body too. A message is selected before its body has downloaded —
        // that is the whole point of fetching lazily — so the reader renders an empty
        // frame first and needs telling when the content actually arrives. Without this
        // the body appeared only if you clicked away and back.
        //
        // Only the bodies that changed. This used to invalidate `['messageBody']` whole,
        // so *any* message updating anywhere re-rendered whatever was open — and rendering
        // a body means fetching every remote image in it. A newsletter left on screen was
        // re-downloaded on every sync tick. The key carries the id, so naming it here is a
        // prefix match over both the images-on and images-off variants of that one message.
        //
        // And only the ones still *waiting*. Naming the id was not narrow enough, because the
        // commonest `messages:updated` of all is the message being marked read 700ms after it
        // opens — so opening a message rendered it, marked it read, and rendered it again.
        // Measured from this install's own log: 191 of 402 renders were the same message
        // repeated within ten seconds, at a p25 gap of 0.696s, which is `DWELL_MS` to three
        // decimal places. That was 146 MB of the 275 MB ever rendered — over half the work the
        // body path has ever done, thrown away.
        //
        // Safe because a body that has arrived cannot change: `sync::bodies` writes
        // `body_state = 'full'` in the same UPDATE as `body_html`, and the engine only fetches
        // a body when `body_state != 'full'`. So an empty `html` means "still coming" and a
        // non-empty one is final. The flags on the message change constantly; the rendered
        // document does not.
        for (const id of ids) {
          void client.invalidateQueries({
            queryKey: ['messageBody', id],
            predicate: (query) => {
              const body = query.state.data as Rendered | undefined
              return body === undefined || body.html === ''
            },
          })
        }
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
    // A rendered body is the largest thing this app puts in the query cache, and the
    // default retention is a clock with no ceiling on count or bytes. On the install this
    // was measured on, the worst five-minute window held **eleven distinct bodies totalling
    // 19.3 MB** — against a renderer whose real private working set is about 34 MB. The
    // sizes are not typical of the mail: a 76 KB stored message rendered to 12.98 MB once
    // its remote images were inlined as base64.
    //
    // A minute keeps what a reader actually returns to — clicking away and back, or
    // arrowing down and up — and drops the long tail of everything merely passed through.
    // The cost of getting it wrong is one re-render, which is now much cheaper than it was.
    gcTime: 60_000,
    queryFn: () => ipc.messageBody(messageId ?? 0, loadRemote),
    enabled: messageId !== null,
    // Bodies are immutable once downloaded; the only thing that changes is whether we have
    // one yet, and the event above covers that.
    staleTime: Infinity,
  })
}
