import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'

import type * as Ipc from '@/lib/ipc'
import type { OutgoingMessage } from '@/lib/generated/OutgoingMessage'
import type { ReplyDraft } from '@/lib/generated/ReplyDraft'

/**
 * A compose window, from the moment it opens: what it was given, what the user changed, and what
 * the last keystroke before sending did. The 2026-11-14 audit's M45, M46 and L25.
 *
 * Component tests, for the reason `composeClose.test.tsx` gives: the window does not initialise
 * outside Tauri.
 */

let requestClose: (() => boolean) | null = null

const send = vi.fn<(message: OutgoingMessage) => Promise<number>>(() => Promise.resolve(1))
const saveDraft = vi.fn(() => Promise.resolve({ id: 1, messageId: 'm1' }))

const ACCOUNTS = [
  { id: 1, displayName: 'Work', email: 'ada@work.test', provider: 'other', color: null },
  { id: 2, displayName: 'Home', email: 'ada@home.test', provider: 'other', color: null },
]

const SIGNATURES: Record<number, string> = {
  1: '<p>Ada, at work</p>',
  2: '<p>Ada, at home</p>',
}

function blank(accountId: number): ReplyDraft {
  return {
    accountId,
    to: [],
    cc: [],
    subject: '',
    quotedHtml: '',
    inReplyTo: null,
    references: [],
    signatureHtml: SIGNATURES[accountId] ?? '',
    signaturePlacement: 'above',
    attachments: [],
  }
}

const REPLY: ReplyDraft = {
  ...blank(1),
  to: [{ name: 'Grace Hopper', email: 'grace@example.test' }],
  subject: 'Re: Figures',
  quotedHtml: '<p>On Monday, Grace wrote:</p><blockquote>The figures are in.</blockquote>',
  inReplyTo: '<figures@example.test>',
  references: ['<figures@example.test>'],
}

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  runningInTauri: true,
  onCloseRequested: (handler: () => boolean) => {
    requestClose = handler
    return Promise.resolve(() => undefined)
  },
  onFileDrop: () => Promise.resolve(() => undefined),
  closeThisWindow: () => Promise.resolve(),
  composeBlank: (accountId: number) => Promise.resolve(blank(accountId)),
  composeReply: () => Promise.resolve(REPLY),
  composeSaveDraft: () => saveDraft(),
  composeSend: (message: OutgoingMessage) => send(message),
  composeUndoSeconds: () => Promise.resolve(10),
  composePickFiles: () => Promise.resolve([]),
  composeSizeLimit: () => Promise.resolve(25_000_000),
  accountsList: () => Promise.resolve(ACCOUNTS),
  contactsSuggest: () => Promise.resolve([]),
  signatureGet: () => Promise.resolve({ html: '', placement: 'above' }),
  onOutboxProgress: () => Promise.resolve(() => undefined),
  storeNow: () => Promise.resolve(),
}))

// The compose module graph is large and its first transform is slow; paid here, not inside a
// timed test. See composeClose.test.tsx.
beforeAll(async () => {
  await import('@/features/compose/ComposeWindow')
}, 60_000)

afterEach(() => {
  requestClose = null
  vi.clearAllMocks()
  vi.resetModules()
  window.history.replaceState({}, '', '/')
})

async function open(search = '') {
  window.history.replaceState({}, '', `/${search}`)
  const { ComposeWindow } = await import('@/features/compose/ComposeWindow')
  render(<ComposeWindow />)
  await waitFor(() => {
    expect(requestClose).not.toBeNull()
  })
}

function body(): HTMLElement {
  return screen.getByRole('textbox', { name: 'Message body' })
}

describe('a reply nobody has touched', () => {
  it('closes without asking to save it', async () => {
    // L25: it arrives with a recipient, a subject and a quote, so it always "had content".
    await open('?message=5')
    await waitFor(() => {
      expect(body().textContent).toContain('The figures are in.')
    })

    expect(requestClose?.()).toBe(true)
  })

  it('is not saved as a draft when the window loses focus', async () => {
    await open('?message=5')
    await waitFor(() => {
      expect(body().textContent).toContain('The figures are in.')
    })

    window.dispatchEvent(new Event('blur'))
    await new Promise((resolve) => setTimeout(resolve, 50))

    expect(saveDraft).not.toHaveBeenCalled()
  })

  it('asks once something has been changed', async () => {
    await open('?message=5')
    await userEvent.type(await screen.findByPlaceholderText('Subject'), ' and totals')

    expect(requestClose?.()).toBe(false)
  })
})

describe('a new message with only its signature in it', () => {
  it('closes without asking to save it', async () => {
    await open()
    await waitFor(() => {
      expect(body().textContent).toContain('Ada, at work')
    })

    expect(requestClose?.()).toBe(true)
  })
})

describe('changing From', () => {
  it('puts the new account’s signature in place of the old one', async () => {
    // M46: the first account's signature stayed in the body whichever account sent it.
    await open()
    await waitFor(() => {
      expect(body().textContent).toContain('Ada, at work')
    })

    await userEvent.selectOptions(await screen.findByRole('combobox', { name: 'Send from' }), '2')

    await waitFor(() => {
      expect(body().textContent).toContain('Ada, at home')
    })
    expect(body().textContent).not.toContain('Ada, at work')
  })
})

describe('Ctrl+Enter in a recipient field', () => {
  it('sends to the address typed there, not only to the ones already committed', async () => {
    // M45: the same Enter committed the address as a chip, in a state change the send could not
    // see yet, so the message went without it.
    await open()
    const to = within(screen.getByRole('group', { name: 'To:' })).getByRole('textbox')

    await userEvent.type(to, 'grace@example.test')
    await userEvent.keyboard('{Control>}{Enter}{/Control}')

    await waitFor(() => {
      expect(send).toHaveBeenCalled()
    })
    expect(send.mock.calls[0]?.[0].to).toEqual([{ name: null, email: 'grace@example.test' }])
  })
})
