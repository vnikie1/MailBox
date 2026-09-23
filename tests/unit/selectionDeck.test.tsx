import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'

import type * as Ipc from '@/lib/ipc'
import type { MessageFull } from '@/lib/generated/MessageFull'
import type { Rendered } from '@/lib/generated/Rendered'

/**
 * Which messages the reader's selection stack draws, and what each sheet is allowed to show.
 *
 * The *geometry* — sheets the size of the pane, each header clear of the sheet in front — is
 * pinned in `tests/e2e/selectionDeck.spec.ts`, because jsdom has no layout. What is testable here
 * is every decision that is not a measurement:
 *
 * - **The cap.** `selectedMessageIds` is the whole selection, and Ctrl+A in a real mailbox puts a
 *   hundred thousand ids in it. A stack that fetched all of them to draw three sheets would be a
 *   freeze, in the one path nobody exercises by hand.
 * - **Remote images stay off.** The front sheet renders the real message, and loading its images
 *   would tell the sender it had been opened. It has only been selected.
 * - **Only the front sheet has a body.** The ones behind show a header strip and nothing else.
 */

const asked: number[] = []
const bodies: { id: number; loadRemote: boolean }[] = []
const ensured: { accountId: number; ids: number[] }[] = []
let bodyHtml = '<p>the body</p>'

function message(id: number, fromName: string): MessageFull {
  return {
    id,
    threadId: id,
    mailboxId: 1,
    accountId: 7,
    subject: `Subject ${String(id)}`,
    fromName,
    fromAddr: `${fromName.toLowerCase()}@example.test`,
    toJson: null,
    ccJson: null,
    dateSent: 1_700_000_000,
    dateReceived: 1_700_000_000,
    size: 0,
    preview: `Preview ${String(id)}`,
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

/** id 99 is the message that has been deleted out from under the selection. */
const NAMES: Record<number, string> = { 1: 'Ada', 2: 'Bram', 3: 'Chen', 4: 'Dev', 5: 'Eli' }

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  messageGet: (id: number): Promise<MessageFull | null> => {
    asked.push(id)
    const name = NAMES[id]
    return Promise.resolve(name === undefined ? null : message(id, name))
  },
  messageBody: (id: number, loadRemote: boolean): Promise<Rendered> => {
    bodies.push({ id, loadRemote })
    return Promise.resolve({
      html: bodyHtml,
      css: '',
      blockedRemote: 0,
      loadedRemote: 0,
      failedRemote: 0,
      inlined: 0,
      fromPlainText: false,
    })
  },
  bodiesEnsure: (accountId: number, ids: number[]): Promise<void> => {
    ensured.push({ accountId, ids })
    return Promise.resolve()
  },
}))

// The frame is a sandboxed iframe that jsdom cannot lay out, and it has its own tests. Here it
// only needs to say where it was put and what it was given.
vi.mock('@/features/reader/MessageFrame', () => ({
  MessageFrame: ({ html, hideBlockedImages }: { html: string; hideBlockedImages?: boolean }) => (
    <div data-testid="frame" data-hides-blocked={String(hideBlockedImages === true)}>
      {html}
    </div>
  ),
}))

async function stack(ids: number[]): Promise<void> {
  const { SelectionDeck } = await import('@/features/reader/SelectionDeck')
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )

  render(<SelectionDeck ids={ids} />, { wrapper })
}

/** The sheets in DOM order, which is deepest first. */
function sheets(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('[data-sheets] > article'))
}

function sheet(index: number): HTMLElement {
  const found = sheets()[index]
  if (found === undefined) throw new Error(`no sheet at ${String(index)}`)
  return found
}

async function settled(): Promise<void> {
  await waitFor(() => {
    expect(screen.getByText('Ada')).toBeInTheDocument()
  })
}

afterEach(() => {
  asked.length = 0
  bodies.length = 0
  ensured.length = 0
  bodyHtml = '<p>the body</p>'
  vi.clearAllMocks()
})

describe('the selection stack', () => {
  it('fetches three messages however many are selected', async () => {
    await stack([1, 2, 3, 4, 5])
    await settled()

    expect(sheets()).toHaveLength(3)
    expect([...asked].sort((a, b) => a - b)).toEqual([1, 2, 3])
  })

  it('counts the selection, not the sheets', async () => {
    await stack([1, 2, 3, 4, 5])

    // A stack that said "3 Messages Selected" over five lit rows would understate what Delete
    // is about to take.
    expect(screen.getByRole('status')).toHaveTextContent('5 Messages Selected')
  })

  it('puts the first selected message on top and the third deepest', async () => {
    await stack([1, 2, 3])
    await settled()

    expect(sheet(2)).toHaveClass('front')
    expect(sheet(1)).toHaveClass('behind1')
    expect(sheet(0)).toHaveClass('behind2')

    expect(sheet(2)).toHaveTextContent('Ada')
    expect(sheet(1)).toHaveTextContent('Bram')
    expect(sheet(0)).toHaveTextContent('Chen')
  })

  it('names every message with its sender and its subject', async () => {
    await stack([1, 2, 3])
    await settled()

    for (const [index, id] of [3, 2, 1].entries()) {
      expect(sheet(index)).toHaveTextContent(NAMES[id] ?? '')
      expect(sheet(index)).toHaveTextContent(`Subject ${String(id)}`)
    }
  })

  it('gives a body to the front sheet and to no other', async () => {
    await stack([1, 2, 3])
    await settled()

    await waitFor(() => {
      expect(screen.getAllByTestId('frame')).toHaveLength(1)
    })

    expect(within(sheet(2)).getByTestId('frame')).toHaveTextContent('the body')

    // Behind, a header and nothing under it. Preview text there showed as a line cut lengthways
    // by the tilted edge of the sheet in front.
    for (const index of [0, 1]) {
      expect(sheet(index)).not.toHaveTextContent(/Preview \d/)
      expect(within(sheet(index)).queryByTestId('frame')).toBeNull()
    }
  })

  it('asks for the body with remote images off, whatever the setting', async () => {
    await stack([1, 2, 3])
    await settled()

    await waitFor(() => {
      expect(bodies.length).toBeGreaterThan(0)
    })

    // Loading them tells the sender the message was opened. It has only been selected.
    expect(bodies.every((body) => !body.loadRemote)).toBe(true)
    expect(bodies.map((body) => body.id)).toEqual([1])
  })

  it('draws the images it withheld as space rather than as broken glyphs', async () => {
    await stack([1, 2, 3])
    await settled()

    // There is no banner over a sheet to say what is missing or to offer to load it, so a
    // broken-image glyph would read as the app failing at something it declined to do.
    await waitFor(() => {
      expect(screen.getByTestId('frame')).toHaveAttribute('data-hides-blocked', 'true')
    })
  })

  it('downloads the front message body, and only that one', async () => {
    await stack([1, 2, 3])
    await settled()

    await waitFor(() => {
      expect(ensured).toEqual([{ accountId: 7, ids: [1] }])
    })
  })

  it('shows the stored preview on the front sheet until the body has arrived', async () => {
    // An empty body is what the core returns while it is still downloading. A blank white frame
    // in that gap reads as the app having broken.
    bodyHtml = ''
    await stack([1, 2, 3])
    await settled()

    await waitFor(() => {
      expect(sheet(2)).toHaveTextContent('Preview 1')
    })
    expect(screen.queryByTestId('frame')).toBeNull()
  })

  it('keeps a sheet for a message that has been deleted under the selection', async () => {
    await stack([1, 99, 3])
    await settled()

    // Three sheets, one of them blank. Dropping it would reshuffle the stack as the list
    // reconciles a moment later — standing rule 6, in a pane with nothing else in it.
    expect(sheets()).toHaveLength(3)
    expect(sheet(1).textContent).toBe('')
  })

  it('reserves room for exactly the sheets it draws', async () => {
    await stack([1, 2])
    await settled()

    expect(sheets()).toHaveLength(2)
    expect(document.querySelector('[data-sheets]')).toHaveAttribute('data-sheets', '2')
  })
})
