import { act, fireEvent, render, screen } from '@testing-library/react'
import { useEffect } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type * as Ui from '@/ui'
import type { Signature } from '@/lib/generated/Signature'

/**
 * Settings → Signatures never writes one account's signature into another.
 *
 * H18 in the 2026-11-14 audit. Switching the account picker re-rendered the pane once with the
 * *previous* account's signature still in state — it was only cleared by an effect afterwards —
 * and the editor, keyed by the new account, mounted with that HTML. The editor reports the
 * content it loads as a change, so the pane saved it to the new account 800ms later, replacing
 * whatever signature that account had.
 */

const saved: { accountId: number; html: string }[] = []

const SIGNATURES: Record<number, Signature> = {
  1: { html: 'Ada, Work', placement: 'below' },
  // No signature yet: the case in the report. A signature to load would have reset the save
  // timer by itself, and hidden the bug.
  2: { html: '', placement: 'below' },
}

vi.mock('@/lib/ipc', () => ({
  signatureGet: (accountId: number): Promise<Signature> =>
    Promise.resolve(SIGNATURES[accountId] ?? { html: '', placement: 'below' }),
  signatureSet: (accountId: number, html: string): Promise<void> => {
    saved.push({ accountId, html })
    return Promise.resolve()
  },
}))

vi.mock('@/app/queries', () => ({
  useAccounts: () => ({
    data: [
      { id: 1, email: 'ada@work.test' },
      { id: 2, email: 'ada@home.test' },
    ],
  }),
}))

/** What the user types into the editor that is up, as the real one would report it. */
let typeIntoEditor: ((html: string) => void) | null = null

// The real editor reports the HTML it loads as a change, because it loads it with an ordinary
// update, and says that change was its own. This stands in for exactly that, and for typing.
vi.mock('@/features/compose/Editor', () => ({
  Editor: ({
    initialHtml = '',
    onChange,
  }: {
    initialHtml?: string
    onChange: (html: string, text: string, origin: 'user' | 'load') => void
  }) => {
    typeIntoEditor = (html) => {
      onChange(html, html, 'user')
    }
    useEffect(() => {
      if (initialHtml !== '') onChange(initialHtml, initialHtml, 'load')
      // Once, on mount, as the real loader does.
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [])
    return <div data-testid="editor">{initialHtml}</div>
  },
}))

// A native picker, so the test can change the account without driving a popup.
vi.mock('@/ui', async (importOriginal) => ({
  ...(await importOriginal<typeof Ui>()),
  Select: ({
    id,
    value,
    options,
    onValueChange,
  }: {
    id: string
    value: number | null
    options: { value: number; label: string }[]
    onValueChange: (value: number) => void
  }) => (
    <select
      id={id}
      aria-label="Account"
      value={value ?? ''}
      onChange={(event) => {
        onValueChange(Number(event.target.value))
      }}
    >
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  ),
}))

beforeEach(() => {
  saved.length = 0
  vi.useFakeTimers({ shouldAdvanceTime: true })
})

afterEach(() => {
  vi.useRealTimers()
})

describe('SignatureSettings', () => {
  it('does not save one account’s signature into an account that has none', async () => {
    const { SignatureSettings } = await import('@/features/settings/SignatureSettings')
    render(<SignatureSettings />)

    expect(await screen.findByText('Ada, Work')).toBeTruthy()

    fireEvent.change(screen.getByLabelText('Account'), { target: { value: '2' } })

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000)
    })

    expect(saved.filter((save) => save.accountId === 2 && save.html.includes('Work'))).toEqual([])
    expect(screen.getByTestId('editor').textContent).toBe('')
  })

  it('does not save a signature back just because it was shown', async () => {
    // M47: loading dropped the signature's colour and size, and the pane then saved what it had
    // loaded, so opening Signatures was enough to strip them for good.
    const { SignatureSettings } = await import('@/features/settings/SignatureSettings')
    render(<SignatureSettings />)

    expect(await screen.findByText('Ada, Work')).toBeTruthy()
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000)
    })

    expect(saved).toEqual([])
  })

  it('saves the last of what was typed when the pane is left straight away', async () => {
    // L24: the save waits 0.8s for typing to stop, and leaving the pane cancelled it.
    const { SignatureSettings } = await import('@/features/settings/SignatureSettings')
    const { unmount } = render(<SignatureSettings />)
    expect(await screen.findByText('Ada, Work')).toBeTruthy()

    act(() => {
      typeIntoEditor?.('Ada, Work. Sent from the train')
    })
    unmount()

    expect(saved).toEqual([{ accountId: 1, html: 'Ada, Work. Sent from the train' }])
  })
})
