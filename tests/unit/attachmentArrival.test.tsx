import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'

import type { MessageFull } from '@/lib/generated/MessageFull'

/**
 * The open conversation refetches when a body arrives, because that is when attachments appear.
 *
 * ## The bug this pins
 *
 * Reported from using the app: "a mail with attachment doesn't load the attachment on the first
 * load of the mail. I have to go to other mails first and do a back and forth and then after
 * some time it loads."
 *
 * A message's attachments do not exist until its body has been downloaded and parsed —
 * `sync::bodies::persist` writes the body and the attachment rows in one transaction, and until
 * then the message row is real and its attachment list is empty. Bodies are fetched lazily, on
 * purpose, so a message opened as it arrives is routinely read before that happens.
 *
 * The reader takes its attachments from `MessageFull` on the thread query. `messages:updated`
 * invalidated the list, the message and the rendered body — and not the thread. So the body
 * appeared and the attachment did not.
 *
 * "After some time" is what identifies it: `mailbox:changed` *does* invalidate `['thread']`, so
 * the next sync tick fixed it by accident, which made a deterministic bug look intermittent.
 *
 * ## Why it drives the real hook
 *
 * The obvious version of this test reimplements the invalidation in the test file and asserts
 * that *that* works — which proves TanStack behaves, and would still pass with the fix deleted
 * from the app. This captures the handler `useMailEvents` actually registers and calls it, so
 * the thing under test is the shipped code.
 */

const listeners: { messagesUpdated: ((ids: number[]) => void)[] } = { messagesUpdated: [] }

vi.mock('@/lib/ipc', () => ({
  onMailboxChanged: () => Promise.resolve(() => undefined),
  onMessagesUpdated: (handler: (ids: number[]) => void) => {
    listeners.messagesUpdated.push(handler)
    return Promise.resolve(() => undefined)
  },
}))

function message(id: number, attachments: number): MessageFull {
  return {
    id,
    threadId: 1,
    mailboxId: 1,
    accountId: 1,
    subject: 'Credit Advice',
    fromName: null,
    fromAddr: 'bank@example.test',
    toJson: null,
    ccJson: null,
    dateSent: 0,
    dateReceived: 0,
    size: 0,
    preview: null,
    seen: false,
    answered: false,
    flagged: false,
    flagColor: null,
    isJunk: false,
    junkByUser: false,
    junkScore: null,
    attachments: Array.from({ length: attachments }, (_, index) => ({
      id: index + 1,
      filename: 'statement.pdf',
      mime: 'application/pdf',
      size: 37_000,
      isInline: false,
    })),
  }
}

/** Mounts the real hook and hands back the client its invalidations land in. */
async function mounted(): Promise<QueryClient> {
  const client = new QueryClient()
  const { useMailEvents } = await import('@/app/queries')

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )

  renderHook(
    () => {
      useMailEvents()
    },
    { wrapper },
  )
  await waitFor(() => {
    expect(listeners.messagesUpdated.length).toBeGreaterThan(0)
  })

  return client
}

function bodiesArrived(ids: number[]): void {
  for (const handler of listeners.messagesUpdated) handler(ids)
}

beforeEach(() => {
  listeners.messagesUpdated = []
  vi.clearAllMocks()
})

describe('when a body finishes downloading', () => {
  it('refetches the conversation the message is in, so its attachments appear', async () => {
    const client = await mounted()

    // The reader opened message 7 before its body arrived: the row is real, and it has no
    // attachments yet. The key is the *selected* message id.
    client.setQueryData(['thread', 7], [message(7, 0)])

    bodiesArrived([7])

    await waitFor(() => {
      expect(client.getQueryState(['thread', 7])?.isInvalidated).toBe(true)
    })
  })

  it('matches on the messages in the thread, not on the key', async () => {
    // The key is whichever message the reader is showing; the data holds the whole
    // conversation. A body arriving for the second message of an open thread has an id that
    // appears nowhere in the key, and matching by key would have missed it entirely.
    const client = await mounted()
    client.setQueryData(['thread', 7], [message(7, 1), message(8, 0)])

    bodiesArrived([8])

    await waitFor(() => {
      expect(client.getQueryState(['thread', 7])?.isInvalidated).toBe(true)
    })
  })

  it('leaves conversations that did not change alone', async () => {
    // The commonest `messages:updated` of all is a message being marked read 700ms after it
    // opens. Refetching every cached conversation on that would undo the care taken elsewhere
    // in queries.ts to stop exactly that kind of waste.
    const client = await mounted()
    client.setQueryData(['thread', 7], [message(7, 1)])
    client.setQueryData(['thread', 99], [message(99, 1)])

    bodiesArrived([7])

    await waitFor(() => {
      expect(client.getQueryState(['thread', 7])?.isInvalidated).toBe(true)
    })
    expect(client.getQueryState(['thread', 99])?.isInvalidated).toBe(false)
  })
})
