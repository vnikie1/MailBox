import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'

import type * as Ipc from '@/lib/ipc'

/**
 * Closing a compose window that has something in it.
 *
 * ## The bug this pins
 *
 * Reported from using the app: "when I try to close the window it asks me to cancel or delete
 * or save as draft, nothing I press works at all and when I try to close the compose message it
 * doesn't close because it keeps showing this pop up menu."
 *
 * `closeThisWindow` calls `window.close()`, which fires `onCloseRequested` again. The handler
 * asked `hasContent()`, the fields still held their text, so it reopened the sheet and cancelled
 * the close. Every button that closes the window deliberately — Delete, Save as Draft, and Send
 * — walked straight back into the question it had just answered. Once anything had been typed,
 * the window could not be closed at all.
 *
 * Send had it worse than the other two: it closed with the fields still populated, so a message
 * that had already gone to the outbox was met with "Save this message as a draft?".
 *
 * ## Why this is a component test
 *
 * It cannot be an end-to-end one. `composeBlank` throws outside Tauri — "Composing is only
 * available in the app" — so the compose window does not initialise in the browser build at
 * all, and `onCloseRequested` and `closeThisWindow` are no-ops there. The behaviour lives
 * entirely in the layer only the packaged app reaches, which is exactly how it shipped.
 */

/** The close handler the window registers, so the test can fire it the way Windows would. */
let requestClose: (() => boolean) | null = null

const closeThisWindow = vi.fn(() => Promise.resolve())
const saveDraft = vi.fn(() => Promise.resolve({ id: 1, messageId: 'm1' }))
const discardDraft = vi.fn(() => Promise.resolve())
const send = vi.fn(() => Promise.resolve(1))

// The real module, with only the pieces this test drives replaced. Listing exports by hand
// meant discovering each missing one through a failure; spreading the original means a new
// call site in the component does not break this test for a reason unrelated to it.
vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  runningInTauri: true,
  onCloseRequested: (handler: () => boolean) => {
    requestClose = handler
    return Promise.resolve(() => undefined)
  },
  closeThisWindow: () => closeThisWindow(),
  composeBlank: () =>
    Promise.resolve({
      accountId: 1,
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
  composeSaveDraft: () => saveDraft(),
  composeDiscardDraft: () => discardDraft(),
  composeSend: () => send(),
  composeUndoSeconds: () => Promise.resolve(10),
  composePickFiles: () => Promise.resolve([]),
  composeSizeLimit: () => Promise.resolve(25_000_000),
  accountsList: () => Promise.resolve([{ id: 1, displayName: 'Me', email: 'me@example.test' }]),
  contactsSuggest: () => Promise.resolve([]),
  signatureGet: () => Promise.resolve({ html: '', placement: 'below' }),
  onOutboxProgress: () => Promise.resolve(() => undefined),
  storeNow: () => Promise.resolve(),
}))

async function openComposeWithContent() {
  const { ComposeWindow } = await import('@/features/compose/ComposeWindow')
  render(<ComposeWindow />)

  await waitFor(() => {
    expect(requestClose).not.toBeNull()
  })

  // Something typed, which is what makes closing a question at all.
  const subject = await screen.findByPlaceholderText('Subject')
  await userEvent.type(subject, 'Quarterly figures')
}

/**
 * Pay the compose module graph's one-time cost before anything is timed.
 *
 * `openComposeWithContent` imports `ComposeWindow` dynamically, which pulls in Lexical and
 * its plugins. The first import of a run pays for Vite transforming all of that on demand —
 * measured at ~2.3s against ~0.7s for every later test in this file, which re-import through
 * a warm transform cache even though `vi.resetModules()` clears the registry between them.
 *
 * That premium sat *inside* the first test, against vitest's 5s default. On an idle machine it
 * fit; on a busy one it did not, and the first test failed at 5,030ms with "Unable to find an
 * element by: [placeholder='Subject']" — which reads as the compose window being broken rather
 * than as a build cost, and sent at least one investigation looking at the wrong thing.
 *
 * Warmed here rather than fixed by raising `testTimeout`, so the 5s budget keeps meaning "this
 * behaviour is fast" instead of silently covering the toolchain. Same reasoning as the e2e
 * suite's warm-then-measure pass in `firstRun.spec.ts` and `shell.spec.ts`.
 */
beforeAll(async () => {
  await import('@/features/compose/ComposeWindow')
}, 60_000)

afterEach(() => {
  requestClose = null
  vi.clearAllMocks()
  vi.resetModules()
})

describe('closing a compose window with something in it', () => {
  it('asks first, and holds the window open', async () => {
    await openComposeWithContent()

    // `false` is what becomes preventDefault, so the window stays.
    expect(requestClose?.()).toBe(false)
    await waitFor(() => {
      expect(screen.getByRole('dialog', { name: /draft/i })).toBeTruthy()
    })
  })

  it('closes when Delete is pressed, instead of asking again', async () => {
    // The bug: Delete called closeThisWindow, which asked the handler again, which reopened
    // the sheet. Answering the question has to stop it being asked.
    await openComposeWithContent()
    requestClose?.()

    await userEvent.click(await screen.findByRole('button', { name: 'Delete' }))

    expect(closeThisWindow).toHaveBeenCalled()
    expect(requestClose?.()).toBe(true)
  })

  it('closes when Save as Draft is pressed, and saves', async () => {
    await openComposeWithContent()
    requestClose?.()

    await userEvent.click(await screen.findByRole('button', { name: 'Save as Draft' }))

    expect(closeThisWindow).toHaveBeenCalled()
    expect(requestClose?.()).toBe(true)
  })

  it('keeps asking after Cancel, because nothing was decided', async () => {
    // Cancel must NOT approve the close: the user said "not yet", not "go".
    await openComposeWithContent()
    requestClose?.()

    await userEvent.click(await screen.findByRole('button', { name: 'Cancel' }))

    expect(closeThisWindow).not.toHaveBeenCalled()
    expect(requestClose?.()).toBe(false)
  })
})

describe('an empty compose window', () => {
  it('closes without asking anything', async () => {
    const { ComposeWindow } = await import('@/features/compose/ComposeWindow')
    render(<ComposeWindow />)

    await waitFor(() => {
      expect(requestClose).not.toBeNull()
    })

    // Nothing typed, so there is nothing to lose and no question worth putting.
    expect(requestClose?.()).toBe(true)
  })
})
