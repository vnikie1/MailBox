import { describe, expect, it } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import { Button, Popover } from '@/ui'

/**
 * Popover keyboard behaviour. Phase 1 exit gate.
 *
 * A popover is not modal — the app behind it stays live — but it still has to behave like
 * a focusable region: focus moves in when it opens, Escape closes it, and focus comes back
 * to the control you opened it from. Leaving focus stranded on a detached element is how a
 * keyboard user ends up back at the top of the document.
 */

function renderPopover() {
  return render(
    <>
      <Popover label="Sender details" trigger={<Button>Details</Button>}>
        <Button>Block sender</Button>
      </Popover>
      <Button>Outside</Button>
    </>,
  )
}

describe('Popover', () => {
  it('opens from the keyboard and names itself', async () => {
    const user = userEvent.setup()
    renderPopover()

    await user.tab()
    expect(screen.getByRole('button', { name: 'Details' })).toHaveFocus()

    await user.keyboard('{Enter}')

    expect(await screen.findByRole('dialog', { name: 'Sender details' })).toBeInTheDocument()
  })

  it('moves focus into the panel when it opens', async () => {
    const user = userEvent.setup()
    renderPopover()

    await user.click(screen.getByRole('button', { name: 'Details' }))
    const dialog = await screen.findByRole('dialog')

    await waitFor(() => {
      expect(dialog).toContainElement(document.activeElement as HTMLElement | null)
    })
  })

  it('closes on Escape and returns focus to the trigger', async () => {
    const user = userEvent.setup()
    renderPopover()

    const trigger = screen.getByRole('button', { name: 'Details' })
    await user.click(trigger)
    await screen.findByRole('dialog')

    await user.keyboard('{Escape}')

    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
    expect(trigger).toHaveFocus()
  })

  it('closes when something outside it is clicked', async () => {
    const user = userEvent.setup()
    renderPopover()

    await user.click(screen.getByRole('button', { name: 'Details' }))
    await screen.findByRole('dialog')

    await user.click(screen.getByRole('button', { name: 'Outside' }))

    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
  })

  it('reports its open state on the trigger', async () => {
    const user = userEvent.setup()
    renderPopover()

    const trigger = screen.getByRole('button', { name: 'Details' })
    expect(trigger).toHaveAttribute('aria-expanded', 'false')

    await user.click(trigger)
    await screen.findByRole('dialog')

    expect(trigger).toHaveAttribute('aria-expanded', 'true')
  })

  it('can be driven as a controlled component', async () => {
    const user = userEvent.setup()
    let openCount = 0

    render(
      <Popover
        label="Sender details"
        trigger={<Button>Details</Button>}
        open={false}
        onOpenChange={() => {
          openCount += 1
        }}
      >
        <span>Never shown</span>
      </Popover>,
    )

    await user.click(screen.getByRole('button', { name: 'Details' }))

    // The parent said closed, so it stays closed — but it did ask.
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(openCount).toBeGreaterThan(0)
  })
})
