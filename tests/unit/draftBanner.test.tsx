import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type * as Ipc from '@/lib/ipc'
import type { MessageFull } from '@/lib/generated/MessageFull'
import { ToastProvider } from '@/ui'

/**
 * The way back into a saved draft.
 *
 * C3 in the 2026-11-14 audit: a draft could be saved and never opened again anywhere in the app.
 * The Drafts mailbox showed it read-only like any other message, and nothing offered to edit it.
 */

const opened: number[] = []

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  composeOpenDraft: (id: number): Promise<string> => {
    opened.push(id)
    return Promise.resolve('compose-1')
  },
}))

vi.mock('@/app/queries', () => ({
  useMailboxes: () => ({
    data: [
      { id: 1, role: 'inbox' },
      { id: 2, role: 'drafts' },
    ],
  }),
}))

function message(mailboxId: number): MessageFull {
  return {
    id: 40,
    threadId: 40,
    mailboxId,
    accountId: 1,
    subject: 'Plan',
    fromName: 'Me',
    fromAddr: 'me@example.test',
    toJson: null,
    ccJson: null,
    dateSent: 0,
    dateReceived: 0,
    size: 0,
    preview: null,
    seen: true,
    answered: false,
    flagged: false,
    flagColor: null,
    isJunk: false,
    junkByUser: false,
    junkScore: null,
    attachments: [],
  }
}

async function show(mailboxId: number) {
  const { DraftBanner } = await import('@/features/reader/DraftBanner')
  render(
    <ToastProvider>
      <DraftBanner message={message(mailboxId)} />
    </ToastProvider>,
  )
}

beforeEach(() => {
  opened.length = 0
})

describe('DraftBanner', () => {
  it('offers to edit a message in Drafts, and opens it', async () => {
    await show(2)

    await userEvent.click(screen.getByRole('button', { name: 'Edit Draft' }))

    expect(opened).toEqual([40])
  })

  it('is not shown anywhere else', async () => {
    await show(1)

    expect(screen.queryByRole('button', { name: 'Edit Draft' })).toBeNull()
  })
})
