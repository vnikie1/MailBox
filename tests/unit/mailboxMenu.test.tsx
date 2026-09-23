import { describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Folder } from 'lucide-react'

import type { MailboxRow } from '@/lib/generated/MailboxRow'
import {
  MailboxContextMenu,
  type MailboxContextMenuProps,
  type MailboxMenuActions,
} from '@/features/sidebar/MailboxContextMenu'
import type { SidebarNode } from '@/features/sidebar/model'
import { Button, Menu } from '@/ui'

/**
 * Which rows the mailbox menu offers, and what each one is greyed for.
 *
 * The rows themselves are driven end to end in `tests/e2e/mailboxMenu.spec.ts`. This covers the
 * states the browser store cannot easily produce: an account that is switched off, one with no
 * Junk mailbox, a Bin that is empty.
 */

function mailbox(overrides: Partial<MailboxRow> = {}): MailboxRow {
  return {
    id: 5,
    accountId: 1,
    displayName: 'Clients',
    parentId: null,
    role: null,
    unreadCount: 3,
    totalCount: 10,
    favouriteOrder: null,
    roleChosen: false,
    delimiter: '/',
    editable: true,
    descendants: 0,
    canContain: true,
    ...overrides,
  }
}

const NODE: SidebarNode = {
  id: 'mailbox-5',
  label: 'Clients',
  icon: Folder,
  mailboxIds: [5],
  accountId: 1,
  unreadCount: 3,
  children: [],
  depth: 0,
}

function actions(): MailboxMenuActions {
  return {
    newMailbox: vi.fn(),
    renameMailbox: vi.fn(),
    deleteMailbox: vi.fn(),
    useAs: vi.fn(),
    setFavourite: vi.fn(),
    exportMailbox: vi.fn(),
    rebuild: vi.fn(),
    eraseDeleted: vi.fn(),
    eraseJunk: vi.fn(),
    markAllRead: vi.fn(),
    synchronise: vi.fn(),
    editAccount: vi.fn(),
    accountInfo: vi.fn(),
  }
}

async function open(props: Partial<MailboxContextMenuProps> = {}) {
  const user = userEvent.setup()
  const handlers = props.actions ?? actions()

  render(
    <Menu label="Mailbox actions" trigger={<Button>Open</Button>}>
      <MailboxContextMenu
        node={NODE}
        mailbox={mailbox()}
        accountName="Google"
        canSync
        canChooseRole
        trash={mailbox({ id: 2, role: 'trash', displayName: 'Bin', totalCount: 4 })}
        junk={mailbox({ id: 3, role: 'junk', displayName: 'Spam', totalCount: 1 })}
        {...props}
        actions={handlers}
      />
    </Menu>,
  )

  await user.click(screen.getByRole('button', { name: 'Open' }))
  const menu = await screen.findByRole('menu', { name: 'Mailbox actions' })
  await waitFor(() => {
    expect(menu.contains(document.activeElement)).toBe(true)
  })

  return { user, menu, handlers }
}

function item(menu: HTMLElement, name: string) {
  return within(menu).getByRole('menuitem', { name })
}

describe('the mailbox menu', () => {
  it('offers everything on a folder the user made, in Mail’s order', async () => {
    const { menu } = await open()

    const labels = within(menu)
      .getAllByRole('menuitem')
      .map((row) => row.textContent)
    expect(labels).toEqual([
      'New Mailbox…',
      'Rename Mailbox…',
      'Delete Mailbox…',
      'Use This Mailbox As',
      'Add to Favourites',
      'Export Mailbox…',
      'Rebuild',
      'Erase Deleted Items…',
      'Erase Junk Mail…',
      'Mark All Messages as Read',
      'Synchronise “Google”',
      'Edit “Google”…',
      'Get Account Info',
    ])
    expect(within(menu).getAllByRole('separator')).toHaveLength(4)
  })

  it('leaves Rename and Delete off a folder the account depends on', async () => {
    const { menu } = await open({
      mailbox: mailbox({ role: 'inbox', displayName: 'Inbox', editable: false }),
      canChooseRole: false,
    })

    expect(within(menu).queryByRole('menuitem', { name: 'Rename Mailbox…' })).toBeNull()
    expect(within(menu).queryByRole('menuitem', { name: 'Delete Mailbox…' })).toBeNull()
    expect(within(menu).queryByRole('menuitem', { name: 'Use This Mailbox As' })).toBeNull()
    expect(item(menu, 'New Mailbox…')).toBeEnabled()
  })

  it('turns Add into Remove for a favourite', async () => {
    const { user, menu, handlers } = await open({ mailbox: mailbox({ favouriteOrder: 2 }) })

    expect(within(menu).queryByRole('menuitem', { name: 'Add to Favourites' })).toBeNull()
    await user.click(item(menu, 'Remove from Favourites'))

    expect(handlers.setFavourite).toHaveBeenCalledWith(expect.objectContaining({ id: 5 }), false)
  })

  it('greys what has nothing to act on, rather than hiding it', async () => {
    const { menu } = await open({
      mailbox: mailbox({ unreadCount: 0 }),
      trash: mailbox({ id: 2, role: 'trash', totalCount: 0 }),
      junk: undefined,
      canSync: false,
    })

    expect(item(menu, 'Erase Deleted Items…')).toBeDisabled()
    expect(item(menu, 'Erase Junk Mail…')).toBeDisabled()
    expect(item(menu, 'Mark All Messages as Read')).toBeDisabled()
    expect(item(menu, 'Synchronise “Google”')).toBeDisabled()
    // A rebuild is done by a sync, so it waits on the same thing.
    expect(item(menu, 'Rebuild')).toBeDisabled()
    // The account rows that need no sync stay available.
    expect(item(menu, 'Edit “Google”…')).toBeEnabled()
    expect(item(menu, 'Get Account Info')).toBeEnabled()
  })

  it('passes each row what it acts on', async () => {
    const handlers = actions()

    for (const [row, handler, expected] of [
      ['New Mailbox…', 'newMailbox', [expect.objectContaining({ id: 5 })]],
      ['Export Mailbox…', 'exportMailbox', [5, 'Clients']],
      ['Rebuild', 'rebuild', [expect.objectContaining({ id: 5 }), 'Clients']],
      ['Erase Deleted Items…', 'eraseDeleted', [1]],
      ['Erase Junk Mail…', 'eraseJunk', [1]],
      ['Mark All Messages as Read', 'markAllRead', [5]],
      ['Synchronise “Google”', 'synchronise', [1]],
      ['Edit “Google”…', 'editAccount', [1]],
      ['Get Account Info', 'accountInfo', [1]],
    ] as const) {
      const { user, menu } = await open({ actions: handlers })
      await user.click(item(menu, row))
      expect(handlers[handler]).toHaveBeenLastCalledWith(...expected)
      cleanup()
    }

    const { user, menu } = await open({ actions: handlers })
    await user.click(item(menu, 'Rename Mailbox…'))
    expect(handlers.renameMailbox).toHaveBeenCalledWith(expect.objectContaining({ id: 5 }))
  })

  it('offers the five roles, ticks the one the mailbox has, and changes only to another', async () => {
    const { user, menu, handlers } = await open({
      mailbox: mailbox({ role: 'trash', displayName: 'Deleted Messages', editable: false }),
    })

    item(menu, 'Use This Mailbox As').focus()
    await user.keyboard('{ArrowRight}')
    const roles = await screen.findByRole('menu', { name: 'Use This Mailbox As' })

    const rows = within(roles).getAllByRole('menuitemcheckbox')
    expect(rows.map((row) => row.textContent)).toEqual(['Drafts', 'Sent', 'Junk', 'Bin', 'Archive'])
    expect(rows.map((row) => row.getAttribute('aria-checked'))).toEqual([
      'false',
      'false',
      'false',
      'true',
      'false',
    ])

    await user.click(within(roles).getByRole('menuitemcheckbox', { name: 'Archive' }))
    expect(handlers.useAs).toHaveBeenCalledWith(expect.objectContaining({ id: 5 }), 'archive')

    // What it is already is not a change.
    cleanup()
    const again = await open({
      mailbox: mailbox({ role: 'trash', displayName: 'Deleted Messages', editable: false }),
      actions: handlers,
    })
    item(again.menu, 'Use This Mailbox As').focus()
    await again.user.keyboard('{ArrowRight}')
    const second = await screen.findByRole('menu', { name: 'Use This Mailbox As' })
    await again.user.click(within(second).getByRole('menuitemcheckbox', { name: 'Bin' }))
    expect(handlers.useAs).toHaveBeenCalledTimes(1)
  })
})
