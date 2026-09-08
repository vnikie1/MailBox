import { FolderInput, Info, MailCheck, RefreshCw, Settings2 } from 'lucide-react'

import type { SidebarNode } from './model'
import { MenuItem, MenuSeparator } from '@/ui'

/**
 * The right-click menu on a mailbox. The order is macOS Mail's own.
 *
 * ## Five rows, against Mail's nine
 *
 * The four that are absent are absent because standing rule 18 forbids a menu item that does
 * nothing, and each is a real gap rather than an oversight:
 *
 *  - **New Mailbox…** — nothing in the app creates a folder, on the server or locally. There is
 *    no IMAP `CREATE`, no sync operation for it, and no command. It also cannot be half-built:
 *    Mail puts Rename and Delete in this same menu, so shipping creation alone would leave the
 *    user with folders they cannot get rid of.
 *  - **Add to Favourites** — Favourites is a fixed list of five with nowhere to store a sixth.
 *    Adding one would also silently renumber Ctrl+1…9, which jump to the *n*th row of
 *    Favourites, and make the shortcuts sheet wrong. Worth doing deliberately, not as a side
 *    effect of a menu.
 *  - **Erase Deleted Items…** and **Erase Junk Mail…** — permanent, undoable by design, and
 *    each needs a command of its own: the only way to enumerate a mailbox from the window
 *    skips snoozed mail, so an Erase driven that way would quietly leave messages behind. Two
 *    new permanent-deletion commands belong in their own change, with their own tests.
 *
 * ## Which rows get a menu at all
 *
 * Only a row backed by exactly one real mailbox in a known account — the same predicate the
 * drop targets use. All Inboxes, All Drafts, All Sent, Flagged and its colours, VIPs and the
 * smart mailboxes get none, because every item here needs a single mailbox or a single account
 * and none of those rows has either.
 */
export interface MailboxContextMenuProps {
  node: SidebarNode
  /** The account's name, for the two rows that quote it. */
  accountName: string
  /** False when the account is switched off in settings, which `sync_now` does not check. */
  syncEnabled: boolean
  actions: MailboxMenuActions
}

export interface MailboxMenuActions {
  exportMailbox: (mailboxId: number, label: string) => void
  markAllRead: (mailboxId: number) => void
  synchronise: (accountId: number) => void
  editAccount: () => void
  accountInfo: (accountId: number) => void
}

export function MailboxContextMenu({
  node,
  accountName,
  syncEnabled,
  actions,
}: MailboxContextMenuProps) {
  const mailboxId = node.mailboxIds[0]
  const accountId = node.accountId

  if (mailboxId === undefined || accountId === undefined) return null

  return (
    <>
      <MenuItem
        label="Export Mailbox…"
        icon={FolderInput}
        onClick={() => {
          actions.exportMailbox(mailboxId, node.label)
        }}
      />

      <MenuSeparator />

      {/* Hidden rather than disabled when there is nothing to do. A permanently greyed row on
          a mailbox that is simply already read is noise; its absence says the same thing. */}
      {node.unreadCount > 0 && (
        <MenuItem
          label="Mark All Messages as Read"
          icon={MailCheck}
          onClick={() => {
            actions.markAllRead(mailboxId)
          }}
        />
      )}

      {node.unreadCount > 0 && <MenuSeparator />}

      {/* Quoted, the way Mail writes it, so a menu opened on the wrong account is obvious
          before anything happens rather than after. */}
      <MenuItem
        label={`Synchronise “${accountName}”`}
        icon={RefreshCw}
        // `sync_now` does not check this the way `sync_all` does, so an account the user
        // switched off could otherwise be started from here.
        disabled={!syncEnabled}
        onClick={() => {
          actions.synchronise(accountId)
        }}
      />
      <MenuItem label={`Edit “${accountName}”…`} icon={Settings2} onClick={actions.editAccount} />

      <MenuSeparator />

      <MenuItem
        label="Get Account Info"
        icon={Info}
        onClick={() => {
          actions.accountInfo(accountId)
        }}
      />
    </>
  )
}
