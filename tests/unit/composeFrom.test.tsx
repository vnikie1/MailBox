import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'

import type * as Ipc from '@/lib/ipc'
import type { DescribedFiles } from '@/lib/generated/DescribedFiles'
import { cannotAttach } from '@/features/compose/attachments'

/**
 * The compose window's From picker and its file drop.
 *
 * Both live only in the packaged app — `composeBlank` refuses outside Tauri, so the window does
 * not initialise in the browser build — which is why they are component tests rather than
 * end-to-end ones. The drop in particular cannot be produced by a browser at all: the paths come
 * from Tauri's native drag-and-drop handler. So the handler is captured here and fired by hand,
 * exactly as the webview would fire it.
 */

type DropHandler = Parameters<typeof Ipc.onFileDrop>[0]

let dropHandler: DropHandler | null = null
const describeFiles = vi.fn<(paths: string[]) => Promise<DescribedFiles>>()

const TWO_ACCOUNTS = [
  { id: 1, displayName: 'Vnikie1', email: 'vnikie1@gmail.com', provider: 'google', color: null },
  { id: 2, displayName: 'Unikie1', email: 'unikie1@yahoo.in', provider: 'yahoo', color: null },
]

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  runningInTauri: true,
  onCloseRequested: () => Promise.resolve(() => undefined),
  onFileDrop: (handler: DropHandler) => {
    dropHandler = handler
    return Promise.resolve(() => {
      dropHandler = null
    })
  },
  composeDescribeFiles: (paths: string[]) => describeFiles(paths),
  composeBlank: (accountId: number) =>
    Promise.resolve({
      accountId,
      to: [],
      cc: [],
      subject: '',
      quotedHtml: '',
      inReplyTo: null,
      references: [],
      signatureHtml: '',
      signaturePlacement: 'below',
      attachments: [],
    }),
  composeUndoSeconds: () => Promise.resolve(10),
  composePickFiles: () => Promise.resolve([]),
  composeSizeLimit: () => Promise.resolve(25_000_000),
  accountsList: () => Promise.resolve(TWO_ACCOUNTS),
  contactsSuggest: () => Promise.resolve([]),
  signatureGet: () => Promise.resolve({ html: '', placement: 'below' }),
  onOutboxProgress: () => Promise.resolve(() => undefined),
  storeNow: () => Promise.resolve(),
}))

// The compose module graph is large and its first transform is slow; paid here, not inside a
// timed test. See composeClose.test.tsx.
beforeAll(async () => {
  await import('@/features/compose/ComposeWindow')
}, 60_000)

afterEach(() => {
  dropHandler = null
  describeFiles.mockReset()
  vi.resetModules()
})

async function openCompose() {
  const { ComposeWindow } = await import('@/features/compose/ComposeWindow')
  render(<ComposeWindow />)
  await waitFor(() => {
    expect(dropHandler).not.toBeNull()
  })
}

function drop(event: Parameters<DropHandler>[0]) {
  act(() => {
    dropHandler?.(event)
  })
}

describe('the From picker', () => {
  it('is the shared popup button, not a hand-built select', async () => {
    // Reported with a screenshot: the list opened white with white text in the dark theme, every
    // account unreadable except the hovered one. The hand-built select this was had
    // `background: none`, and Windows draws the open list from the control's background.
    // `ui/Select` owns an opaque surface and the colours of its options.
    await openCompose()

    const from = await screen.findByRole('combobox', { name: 'Send from' })

    // `Select`'s own structure: the chevron laid over the control. A bare select has none.
    expect(from.parentElement?.querySelector('svg')).not.toBeNull()

    const options = within(from).getAllByRole('option')
    expect(options.map((option) => option.textContent)).toEqual([
      'Vnikie1 — vnikie1@gmail.com',
      'Unikie1 — unikie1@yahoo.in',
    ])
  })

  it('changes the sending account', async () => {
    await openCompose()

    const from = await screen.findByRole('combobox', { name: 'Send from' })
    await waitFor(() => {
      expect(from).toHaveValue('1')
    })

    await userEvent.selectOptions(from, '2')
    expect(from).toHaveValue('2')
  })
})

describe('dropping files on the compose window', () => {
  it('shows a drop target while files are held over the window', async () => {
    await openCompose()
    expect(screen.queryByText('Drop to attach')).toBeNull()

    drop({ type: 'hover' })
    expect(screen.getByText('Drop to attach')).toBeTruthy()

    drop({ type: 'leave' })
    expect(screen.queryByText('Drop to attach')).toBeNull()
  })

  it('attaches what was dropped, by the paths the drop carried', async () => {
    describeFiles.mockResolvedValue({
      files: [
        { path: 'C:\\Docs\\Rent agreement.docx', filename: 'Rent agreement.docx', size: 22_704 },
        { path: 'C:\\Docs\\photo.jpg', filename: 'photo.jpg', size: 1_024 },
      ],
      skipped: [],
    })

    await openCompose()

    drop({ type: 'hover' })
    drop({ type: 'drop', paths: ['C:\\Docs\\Rent agreement.docx', 'C:\\Docs\\photo.jpg'] })

    expect(describeFiles).toHaveBeenCalledWith([
      'C:\\Docs\\Rent agreement.docx',
      'C:\\Docs\\photo.jpg',
    ])
    expect(await screen.findByText('Rent agreement.docx')).toBeTruthy()
    expect(screen.getByText('photo.jpg')).toBeTruthy()

    // The target goes away with the drop, not only on leave.
    expect(screen.queryByText('Drop to attach')).toBeNull()
  })

  it('does not attach the same file twice', async () => {
    // Dropping a file that is already attached is easy to do, and two chips for one path would
    // send it twice — and collide as React keys.
    const file = { path: 'C:\\Docs\\notes.txt', filename: 'notes.txt', size: 12 }
    describeFiles.mockResolvedValue({ files: [file], skipped: [] })

    await openCompose()

    drop({ type: 'drop', paths: [file.path] })
    expect(await screen.findByText('notes.txt')).toBeTruthy()

    drop({ type: 'drop', paths: [file.path] })
    await waitFor(() => {
      expect(describeFiles).toHaveBeenCalledTimes(2)
    })

    expect(screen.getAllByText('notes.txt')).toHaveLength(1)
  })

  it('says so when a dropped folder cannot be attached', async () => {
    describeFiles.mockResolvedValue({
      files: [{ path: 'C:\\Docs\\a.pdf', filename: 'a.pdf', size: 10 }],
      skipped: ['Photos'],
    })

    await openCompose()
    drop({ type: 'drop', paths: ['C:\\Docs\\a.pdf', 'C:\\Photos'] })

    expect(await screen.findByText('a.pdf')).toBeTruthy()
    expect(screen.getByRole('alert').textContent).toBe(cannotAttach(['Photos']))
  })

  it('ignores an empty drop rather than asking the core about nothing', async () => {
    await openCompose()
    drop({ type: 'drop', paths: [] })
    expect(describeFiles).not.toHaveBeenCalled()
  })
})

describe('the wording for items that are not files', () => {
  it('names a single item and counts several', () => {
    expect(cannotAttach(['Photos'])).toBe(
      '“Photos” was not attached. Only files can be attached — not folders.',
    )
    expect(cannotAttach(['Photos', 'Music', 'Old'])).toBe(
      '3 items were not attached. Only files can be attached — not folders.',
    )
  })
})
