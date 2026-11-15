import { render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { ErrorBoundary } from '@/ui'

/**
 * A render error stays in the pane that threw.
 *
 * With no boundary, React unmounts the whole tree on an error thrown during render. The reader
 * had none, so one message whose markup tripped the table repair blanked the entire window —
 * sidebar, list and all — until a restart (M23 in the 2026-11-14 audit).
 */

function Message({ id }: { id: number }) {
  if (id === 2) throw new Error('this message breaks the reader')
  return <p>message {id}</p>
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('ErrorBoundary', () => {
  it('shows the fallback in place of what threw, and leaves its neighbours alone', () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined)

    render(
      <>
        <p>the sidebar</p>
        <ErrorBoundary label="test" resetKey={2} fallback={<p>could not be shown</p>}>
          <Message id={2} />
        </ErrorBoundary>
      </>,
    )

    expect(screen.getByText('could not be shown')).toBeTruthy()
    expect(screen.getByText('the sidebar')).toBeTruthy()
  })

  it('gives the next message a fresh attempt', () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined)

    const { rerender } = render(
      <ErrorBoundary label="test" resetKey={2} fallback={<p>could not be shown</p>}>
        <Message id={2} />
      </ErrorBoundary>,
    )
    expect(screen.getByText('could not be shown')).toBeTruthy()

    rerender(
      <ErrorBoundary label="test" resetKey={3} fallback={<p>could not be shown</p>}>
        <Message id={3} />
      </ErrorBoundary>,
    )

    expect(screen.getByText('message 3')).toBeTruthy()
    expect(screen.queryByText('could not be shown')).toBeNull()
  })

  it('names itself in what it logs', () => {
    const logged = vi.spyOn(console, 'error').mockImplementation(() => undefined)

    render(
      <ErrorBoundary label="message frame" resetKey={2} fallback={null}>
        <Message id={2} />
      </ErrorBoundary>,
    )

    expect(logged.mock.calls.some((call) => call[0] === '[message frame]')).toBe(true)
  })
})
