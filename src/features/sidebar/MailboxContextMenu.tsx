import {
  AtSign,
  Bookmark,
  BookmarkMinus,
  FolderCog,
  FolderMinus,
  FolderOutput,
  FolderPen,
  FolderPlus,
  HardDriveDownload,
  Info,
  MailOpen,
  RefreshCw,
  ShieldX,
  Trash2,
} from 'lucide-react'

import type { MailboxRow } from '@/lib/generated/MailboxRow'
import type { MailboxUse } from '@/lib/ipc'
import { Menu, MenuItem, MenuSeparator } from '@/ui'

import type { SidebarNode } from './model'

/**
 * The right-click menu on a mailbox. docs/01 §3, and the capture of Mail's own menu this was
 * built against:
 *
 *     New Mailbox…
 *     ─────────────
 *     Add to Favourites
 *     Export Mailbox…
 *     ─────────────
 *     Erase Deleted Items…
 *     Erase Junk Mail…
 *     Mark All Messages as Read
 *     ─────────────
 *     Synchronise “Google”
 *     Edit “Google”…
 *     ─────────────
 *     Get Account Info
 *
 * On a folder the user made, Rename Mailbox… and Delete Mailbox… follow New Mailbox…, as they do
 * in Mail — and they have to exist before New Mailbox can: a menu that makes folders and offers
 * no way to get rid of them leaves the user stuck with every typo. On the Inbox and the other
 * folders the account files into, and on Gmail's own, the two rows are absent rather than
 * greyed, because they never apply there; that is what Mail's capture shows on an Inbox.
 *
 * Two rows docs/01 §3 lists and the capture does not show are here too: Use This Mailbox As ▸,
 * after the rows that change the folder itself, and Rebuild, after Export Mailbox…, in the order
 * the spec lists them. Use This Mailbox As is absent where it never applies — the Inbox, Gmail,
 * which decides its own, and an imported archive, which files nothing.
 *
 * ## Greyed, not hidden
 *
 * The rows that act on what a mailbox holds are greyed when there is nothing to act on — an
 * empty Bin, no Junk, nothing unread, an account that does not sync — rather than removed. Mark
 * All Messages as Read used to disappear instead, which made the menu a different shape on every
 * other mailbox; Mail's menu keeps its shape, and a menu that keeps its shape can be used by
 * position.
 *
 * ## Which rows get a menu at all
 *
 * Only a row backed by exactly one real mailbox in a known account — the same predicate the drop
 * targets use (`canOpenMailboxMenu`). All Inboxes, All Drafts, All Sent, Flagged and its colours,
 * VIPs and the smart mailboxes get none: every item here needs a single mailbox or a single
 * account, and none of those rows has either.
 */
export interface MailboxContextMenuProps {
  node: SidebarNode
  /** The mailbox the row stands for. */
  mailbox: MailboxRow
  /** The account's name, for the two rows that quote it. */
  accountName: string
  /**
   * False when the account is switched off in Settings or has no server at all — an imported
   * archive. `sync_now` does not check the first the way `sync_all` does.
   */
  canSync: boolean
  /** Whether Use This Mailbox As applies: a server account that is not Gmail, not the Inbox. */
  canChooseRole: boolean
  /** The account's Bin and Junk mailboxes, when it has them. */
  trash: MailboxRow | undefined
  junk: MailboxRow | undefined
  actions: MailboxMenuActions
}

export interface MailboxMenuActions {
  /** New Mailbox, opened on the mailbox the menu was opened on. */
  newMailbox: (mailbox: MailboxRow) => void
  renameMailbox: (mailbox: MailboxRow) => void
  deleteMailbox: (mailbox: MailboxRow) => void
  useAs: (mailbox: MailboxRow, usage: MailboxUse) => void
  setFavourite: (mailbox: MailboxRow, favourite: boolean) => void
  exportMailbox: (mailboxId: number, label: string) => void
  rebuild: (mailbox: MailboxRow, label: string) => void
  eraseDeleted: (accountId: number) => void
  eraseJunk: (accountId: number) => void
  markAllRead: (mailboxId: number) => void
  synchronise: (accountId: number) => void
  editAccount: (accountId: number) => void
  accountInfo: (accountId: number) => void
}

/** Use This Mailbox As, in Mail's order, in the words the sidebar uses for each. */
const USES: { usage: MailboxUse; label: string }[] = [
  { usage: 'drafts', label: 'Drafts' },
  { usage: 'sent', label: 'Sent' },
  { usage: 'junk', label: 'Junk' },
  { usage: 'trash', label: 'Bin' },
  { usage: 'archive', label: 'Archive' },
]

export function MailboxContextMenu({
  node,
  mailbox,
  accountName,
  canSync,
  canChooseRole,
  trash,
  junk,
  actions,
}: MailboxContextMenuProps) {
  const accountId = mailbox.accountId
  const favourite = mailbox.favouriteOrder !== null

  return (
    <>
      <MenuItem
        label="New Mailbox…"
        icon={FolderPlus}
        onClick={() => {
          actions.newMailbox(mailbox)
        }}
      />
      {mailbox.editable && (
        <MenuItem
          label="Rename Mailbox…"
          icon={FolderPen}
          onClick={() => {
            actions.renameMailbox(mailbox)
          }}
        />
      )}
      {mailbox.editable && (
        <MenuItem
          label="Delete Mailbox…"
          icon={FolderMinus}
          destructive
          onClick={() => {
            actions.deleteMailbox(mailbox)
          }}
        />
      )}
      {canChooseRole && (
        <Menu label="Use This Mailbox As" icon={FolderCog}>
          {USES.map(({ usage, label }) => (
            <MenuItem
              key={usage}
              label={label}
              checked={mailbox.role === usage}
              onClick={() => {
                // Choosing what it already is changes nothing, as in Mail.
                if (mailbox.role !== usage) actions.useAs(mailbox, usage)
              }}
            />
          ))}
        </Menu>
      )}

      <MenuSeparator />

      {/* The one row whose label turns over, the way Mail's does. Two rows, one of them always
          greyed, would say the same thing with twice the noise. */}
      <MenuItem
        label={favourite ? 'Remove from Favourites' : 'Add to Favourites'}
        icon={favourite ? BookmarkMinus : Bookmark}
        onClick={() => {
          actions.setFavourite(mailbox, !favourite)
        }}
      />
      <MenuItem
        label="Export Mailbox…"
        icon={FolderOutput}
        onClick={() => {
          // The row's own label, which for a favourite or a unified child says which account.
          actions.exportMailbox(mailbox.id, node.label)
        }}
      />
      {/* Greyed where Synchronise is: the rebuild is done by a sync, and an account that does
          not sync would keep the request for ever. */}
      <MenuItem
        label="Rebuild"
        icon={HardDriveDownload}
        disabled={!canSync}
        onClick={() => {
          actions.rebuild(mailbox, node.label)
        }}
      />

      <MenuSeparator />

      {/* Greyed on an empty mailbox. The store holds the newest messages of any folder, so an
          empty one here is an empty one on the server — there is nothing an erase could do. */}
      <MenuItem
        label="Erase Deleted Items…"
        icon={Trash2}
        disabled={trash === undefined || trash.totalCount === 0}
        onClick={() => {
          actions.eraseDeleted(accountId)
        }}
      />
      <MenuItem
        label="Erase Junk Mail…"
        icon={ShieldX}
        disabled={junk === undefined || junk.totalCount === 0}
        onClick={() => {
          actions.eraseJunk(accountId)
        }}
      />
      <MenuItem
        label="Mark All Messages as Read"
        icon={MailOpen}
        disabled={mailbox.unreadCount === 0}
        onClick={() => {
          actions.markAllRead(mailbox.id)
        }}
      />

      <MenuSeparator />

      {/* Quoted, the way Mail writes it, so a menu opened on the wrong account is obvious
          before anything happens rather than after. */}
      <MenuItem
        label={`Synchronise “${accountName}”`}
        icon={RefreshCw}
        disabled={!canSync}
        onClick={() => {
          actions.synchronise(accountId)
        }}
      />
      <MenuItem
        label={`Edit “${accountName}”…`}
        icon={AtSign}
        onClick={() => {
          actions.editAccount(accountId)
        }}
      />

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
