import {
  Archive,
  FileText,
  Flag,
  Folder,
  Inbox,
  Mail,
  Send,
  ShieldAlert,
  Star,
  Trash2,
  type LucideIcon,
} from 'lucide-react'

import type { AccountRow } from '@/lib/generated/AccountRow'
import type { MailboxRow } from '@/lib/generated/MailboxRow'

/**
 * The sidebar tree. docs/01 §3.
 *
 * Built from the flat mailbox list the core returns, because the shape is a *view*: "All
 * Inboxes" is not a mailbox, it is every account's inbox presented as one row with the
 * per-account inboxes underneath. `mailboxes_tree` deliberately returns rows rather than a
 * tree for that reason.
 */

export interface SidebarNode {
  id: string
  label: string
  icon: LucideIcon
  /**
   * Which mailboxes selecting this row shows. Empty for rows that are only containers.
   *
   * There is no single `mailboxId` alongside this, and there must not be. There used to be,
   * set to the first of the list, and it caused two bugs at once: "All Inboxes" showed one
   * account's mail rather than the union it promises, and selection keyed off it highlighted
   * every row that happened to share a mailbox.
   */
  mailboxIds: number[]
  unreadCount: number
  children: SidebarNode[]
  depth: number
}

export interface SidebarSection {
  id: string
  title: string
  nodes: SidebarNode[]
}

const ROLE_ICONS: Record<string, LucideIcon> = {
  inbox: Inbox,
  drafts: FileText,
  sent: Send,
  junk: ShieldAlert,
  trash: Trash2,
  archive: Archive,
  flagged: Flag,
  all: Mail,
  vip: Star,
}

/**
 * `role` is an open set — servers invent folder roles — so an unrecognised one degrades to
 * a plain folder icon rather than failing. Standing rule 13's "parse leniently, degrade
 * visibly" applies to metadata as much as to MIME.
 */
function iconFor(role: string | null): LucideIcon {
  if (role === null) return Folder
  return ROLE_ICONS[role] ?? Folder
}

/**
 * A unified row: one role gathered across every account. What makes "All Inboxes" show one
 * number rather than three you have to add up.
 */
function unifiedNode(
  mailboxes: MailboxRow[],
  accounts: AccountRow[],
  id: string,
  label: string,
  role: string,
): SidebarNode {
  const matching = mailboxes.filter((mailbox) => mailbox.role === role)

  return {
    id,
    label,
    icon: iconFor(role),
    mailboxIds: matching.map((mailbox) => mailbox.id),
    unreadCount: matching.reduce((sum, mailbox) => sum + mailbox.unreadCount, 0),
    depth: 0,
    children: matching.map((mailbox) => {
      const account = accounts.find((entry) => entry.id === mailbox.accountId)
      return {
        id: `${id}-${String(mailbox.id)}`,
        label: account?.displayName ?? mailbox.displayName,
        icon: iconFor(role),
        mailboxIds: [mailbox.id],
        unreadCount: mailbox.unreadCount,
        children: [],
        depth: 1,
      }
    }),
  }
}

/** Mail's order within an account, which is not alphabetical. */
const ACCOUNT_ROLE_ORDER = ['inbox', 'drafts', 'sent', 'junk', 'trash', 'archive']

export function buildSidebar(accounts: AccountRow[], mailboxes: MailboxRow[]): SidebarSection[] {
  const favourites: SidebarSection = {
    id: 'favourites',
    title: 'Favourites',
    nodes: [
      unifiedNode(mailboxes, accounts, 'all-inboxes', 'All Inboxes', 'inbox'),
      {
        id: 'flagged',
        label: 'Flagged',
        icon: Flag,
        mailboxIds: [],
        unreadCount: 0,
        children: [],
        depth: 0,
      },
      unifiedNode(mailboxes, accounts, 'all-drafts', 'All Drafts', 'drafts'),
      unifiedNode(mailboxes, accounts, 'all-sent', 'All Sent', 'sent'),
    ],
  }

  const accountSections: SidebarSection[] = accounts.map((account) => {
    const owned = mailboxes.filter((mailbox) => mailbox.accountId === account.id)

    const ordered = [
      ...ACCOUNT_ROLE_ORDER.map((role) => owned.find((mailbox) => mailbox.role === role)).filter(
        (mailbox) => mailbox !== undefined,
      ),
      // Custom folders below the standard set, alphabetically — the one place in the
      // sidebar where alphabetical is right, because the user made these and there is no
      // other order to respect.
      ...owned
        .filter((mailbox) => mailbox.role === null)
        .sort((a, b) => a.displayName.localeCompare(b.displayName)),
    ]

    return {
      id: `account-${String(account.id)}`,
      title: account.displayName,
      // The node id is derived from the mailbox id, which is what lets the shell open on a
      // specific account's inbox rather than on the unified row above it.
      nodes: ordered.map((mailbox) => ({
        id: `mailbox-${String(mailbox.id)}`,
        label: mailbox.displayName,
        icon: iconFor(mailbox.role),
        mailboxIds: [mailbox.id],
        unreadCount: mailbox.unreadCount,
        children: [],
        depth: 0,
      })),
    }
  })

  return [favourites, { id: 'smart', title: 'Smart Mailboxes', nodes: [] }, ...accountSections]
}

/** Flattens the tree to the rows actually on screen, honouring what is collapsed. */
export function visibleRows(nodes: SidebarNode[], collapsed: Set<string>): SidebarNode[] {
  const rows: SidebarNode[] = []

  const walk = (node: SidebarNode) => {
    rows.push(node)
    if (node.children.length > 0 && !collapsed.has(node.id)) {
      node.children.forEach(walk)
    }
  }

  nodes.forEach(walk)
  return rows
}
