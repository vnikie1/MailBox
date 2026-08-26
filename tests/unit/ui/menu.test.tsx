import { describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import { Button, Menu, MenuItem, MenuSeparator } from '@/ui'

/**
 * Menu keyboard behaviour. Phase 1 exit gate.
 *
 * These are the interactions that make a menu feel native rather than like a styled list:
 * arrow keys walk it, letters jump within it, Escape gets you out and puts focus back
 * where it was. A menu that only works with a mouse fails the definition of done's
 * "fully keyboard operable" line outright.
 */

/**
 * Focus moves in a React effect, not in the keydown handler, and userEvent does not flush
 * effects between keystrokes — so a synchronous assertion passes or fails on scheduler
 * interleaving.
 *
 * The timeout is raised above waitFor's 1s default deliberately. These pass in isolation
 * and were still failing inside , where eight test files share the machine;
 * 1s is a load measurement, not a correctness one, and a test that fails on a busy CI box
 * is one people learn to re-run rather than read.
 */
async function expectFocus(element: HTMLElement | undefined) {
  await waitFor(() => {
    expect(element).toHaveFocus()
  })
}

/**
 * Opens a menu and waits until it is ready to take keys.
 *
 * "Ready" means focus has actually reached the panel. The panel mounts, FloatingFocusManager
 * moves focus into it, and FloatingList registers the items — all in effects, and all after
 * `findByRole` can already see the element. An ArrowDown sent in that gap is delivered to a
 * list that has no items registered yet, so it moves nothing and the test then waits out its
 * timeout for focus that is never coming.
 *
 * This was the actual cause of a flake that survived two rounds of "wait longer", including
 * a 4s timeout that only made the failure slower. Waiting for the right thing fixes it;
 * waiting more for the wrong thing never did.
 */
async function openMenu(user: ReturnType<typeof userEvent.setup>, name: string) {
  await user.click(screen.getByRole('button', { name }))
  const menu = await screen.findByRole('menu')

  await waitFor(() => {
    expect(menu.contains(document.activeElement)).toBe(true)
  })

  return menu
}

function renderMenu(onReply = vi.fn()) {
  return {
    onReply,
    ...render(
      <Menu label="Message actions" trigger={<Button>Actions</Button>}>
        <MenuItem label="Reply" onClick={onReply} />
        <MenuItem label="Forward" />
        <MenuSeparator />
        <MenuItem label="Delete" />
      </Menu>,
    ),
  }
}

describe('Menu', () => {
  it('opens from the trigger and names itself for assistive technology', async () => {
    const user = userEvent.setup()
    renderMenu()

    expect(screen.queryByRole('menu')).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Actions' }))

    const menu = await screen.findByRole('menu', { name: 'Message actions' })
    expect(within(menu).getAllByRole('menuitem')).toHaveLength(3)
  })

  it('walks its items with the arrow keys', async () => {
    const user = userEvent.setup()
    renderMenu()

    const menu = await openMenu(user, 'Actions')
    const items = within(menu).getAllByRole('menuitem')

    // The menu opens with nothing highlighted, as macOS does; the first Down highlights
    // the first row rather than the second.
    await user.keyboard('{ArrowDown}')
    await expectFocus(items[0])

    await user.keyboard('{ArrowDown}')
    await expectFocus(items[1])

    await user.keyboard('{ArrowUp}')
    await expectFocus(items[0])
  })

  it('skips the separator rather than landing on it', async () => {
    const user = userEvent.setup()
    renderMenu()

    const menu = await openMenu(user, 'Actions')
    const items = within(menu).getAllByRole('menuitem')

    await user.keyboard('{ArrowDown}')
    await expectFocus(items[0])
    await user.keyboard('{ArrowDown}')
    await expectFocus(items[1])
    await user.keyboard('{ArrowDown}')
    await expectFocus(items[2])
    expect(items[2]).toHaveAccessibleName('Delete')
  })

  it('jumps to an item when you type its first letters', async () => {
    const user = userEvent.setup()
    renderMenu()

    const menu = await openMenu(user, 'Actions')
    const items = within(menu).getAllByRole('menuitem')

    await user.keyboard('de')
    await expectFocus(items[2])
  })

  it('invokes an item with Enter and closes the menu', async () => {
    const user = userEvent.setup()
    const { onReply } = renderMenu()

    const menu = await openMenu(user, 'Actions')

    // Enter has to land on the item, not on the panel. Focus moves in an effect, so
    // pressing both keys in one call races the scheduler — which is what made this fail
    // only inside the full suite.
    await user.keyboard('{ArrowDown}')
    await expectFocus(within(menu).getAllByRole('menuitem')[0])
    await user.keyboard('{Enter}')

    expect(onReply).toHaveBeenCalledOnce()

    // The panel outlives the click by one fade — see useTransitionStatus in Menu.tsx.
    await waitFor(() => {
      expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    })
  })

  it('closes on Escape and returns focus to the trigger', async () => {
    const user = userEvent.setup()
    renderMenu()

    const trigger = screen.getByRole('button', { name: 'Actions' })
    await openMenu(user, 'Actions')

    await user.keyboard('{Escape}')

    await waitFor(() => {
      expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    })
    expect(trigger).toHaveFocus()
  })

  it('reports a checkable item as a checkbox item with its state', async () => {
    const user = userEvent.setup()
    render(
      <Menu label="View" trigger={<Button>View</Button>}>
        <MenuItem label="Flag" checked />
        <MenuItem label="Mark as Unread" checked={false} />
      </Menu>,
    )

    await user.click(screen.getByRole('button', { name: 'View' }))

    expect(await screen.findByRole('menuitemcheckbox', { name: 'Flag' })).toBeChecked()
    expect(screen.getByRole('menuitemcheckbox', { name: 'Mark as Unread' })).not.toBeChecked()
  })

  it('opens a submenu with the right arrow and closes it with the left', async () => {
    const user = userEvent.setup()
    render(
      <Menu label="Message actions" trigger={<Button>Actions</Button>}>
        <MenuItem label="Reply" />
        <Menu label="Move to">
          <MenuItem label="Archive" />
          <MenuItem label="Receipts" />
        </Menu>
      </Menu>,
    )

    await openMenu(user, 'Actions')

    const items = within(screen.getByRole('menu', { name: 'Message actions' })).getAllByRole(
      'menuitem',
    )
    await user.keyboard('{ArrowDown}')
    await expectFocus(items[0])

    await user.keyboard('{ArrowDown}')
    const submenuTrigger = screen.getByRole('menuitem', { name: /Move to/ })
    await expectFocus(submenuTrigger)
    expect(submenuTrigger).toHaveAttribute('aria-haspopup', 'menu')

    await user.keyboard('{ArrowRight}')
    const submenu = await screen.findByRole('menu', { name: 'Move to' })

    await expectFocus(within(submenu).getByRole('menuitem', { name: 'Archive' }))

    await user.keyboard('{ArrowLeft}')
    await waitFor(() => {
      expect(screen.queryByRole('menu', { name: 'Move to' })).not.toBeInTheDocument()
    })
    expect(screen.getByRole('menu', { name: 'Message actions' })).toBeInTheDocument()
  })
})
