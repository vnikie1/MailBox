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
import type { BuiltinFavourite } from '@/lib/generated/BuiltinFavourite'
import type { FavouriteRow } from '@/lib/generated/FavouriteRow'
import type { FlagName } from '@/lib/generated/FlagName'
import type { MailboxRow } from '@/lib/generated/MailboxRow'
import type { Predicate } from '@/lib/generated/Predicate'
import type { SmartMailbox } from '@/lib/generated/SmartMailbox'
import type { Vip } from '@/lib/generated/Vip'
import type { MailboxSelection } from '@/store/mail'

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
  /**
   * The account this row's mailbox belongs to. Set only on rows backed by exactly one real
   * mailbox — a container has none and a unified row spans several, so neither has an answer.
   *
   * Carried so that dropping messages on a row can be refused before it happens: mail cannot
   * move between accounts, and `msg_move` says so with `crossAccount`. Without this the
   * sidebar would light up every folder in every account and let the core do the refusing,
   * which is a toast after the fact rather than a cursor before it.
   */
  accountId?: number
  /**
   * One of the seven flag colours, on the rows under Flagged.
   *
   * The row's icon takes that colour instead of the accent. Without it every colour under
   * Flagged drew the same orange flag, so the one thing those rows exist to tell apart was
   * the one thing they did not show.
   */
  flagColor?: string
  /**
   * The colour of the account this row belongs to, when the user has pinned one.
   *
   * Separate from `flagColor` although both end up tinting the same icon, because they are
   * different questions with different answers: a flag colour says what the row *is*, and
   * an account colour says who it *belongs to*. Collapsing them into one field would make
   * a row under Flagged and a row under an account indistinguishable to the CSS, and the
   * two have to lose to selection in the same way but for different reasons.
   *
   * Set on the rows where telling accounts apart is the point: every mailbox inside an
   * account's own section, and the per-account children of the unified rows, which are
   * labelled by account name and were otherwise seven identical grey inboxes.
   */
  accountColor?: string
  /**
   * Set on rows that are a saved search rather than a folder: smart mailboxes, Flagged, and
   * each flag colour under it. The list queries by this instead of by mailbox id.
   *
   * A row has one or the other, never both. Carrying both would leave two ways to ask the
   * same question and no rule about which wins.
   */
  predicate?: Predicate
  /**
   * The Favourites entry this row is, on the rows of that section, once the store's order has
   * been read. What a drag or Alt+Up and Alt+Down report to `favourite_move`; a row without one
   * does not move.
   */
  favouriteId?: number
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
        accountId: mailbox.accountId,
        // Spread rather than set to `undefined`: with `exactOptionalPropertyTypes` an
        // explicit undefined is not an absent key, and the row branches on absence.
        ...(account?.color == null ? {} : { accountColor: account.color }),
        unreadCount: mailbox.unreadCount,
        children: [],
        depth: 1,
      }
    }),
  }
}

/** Mail's order within an account, which is not alphabetical. */
const ACCOUNT_ROLE_ORDER = ['inbox', 'drafts', 'sent', 'junk', 'trash', 'archive']

/** `is flagged` — the predicate behind the Flagged favourite. */
function flaggedPredicate(): Predicate {
  return { type: 'is', value: { field: 'isFlagged', op: 'isTrue', value: '' } }
}

/**
 * `is flagged, and mentions this colour`.
 *
 * There is deliberately no `flagColor` field on `Field`. Adding one would let a user write a
 * smart mailbox against a colour and then rename that colour out from under themselves, and the
 * mailbox would silently stop matching. These children are built by the app rather than saved,
 * so they can be rebuilt whenever the names change.
 */
function colourPredicate(color: string): Predicate {
  return {
    type: 'all',
    value: [
      flaggedPredicate(),
      { type: 'is', value: { field: 'anyText', op: 'contains', value: color } },
    ],
  }
}

/**
 * VIPs, as a mailbox. docs/01 §8.
 *
 * A saved search over the VIP addresses rather than a folder, so nothing is moved and a VIP's
 * mail still appears in the Inbox where they expect it.
 *
 * Absent when there are no VIPs. An empty row that can never fill is worse than no row: it
 * reads as a feature that is broken rather than one that has not been used.
 */
function vipNode(vips: Vip[]): SidebarNode | null {
  if (vips.length === 0) return null

  return {
    id: 'vips',
    label: 'VIPs',
    icon: Star,
    mailboxIds: [],
    // One `any` over the addresses. Matching on `from` rather than `anyText` so a message that
    // merely *mentions* a VIP does not qualify — the row means "from these people".
    predicate: {
      type: 'any',
      value: vips.map((vip) => ({
        type: 'is' as const,
        value: { field: 'from' as const, op: 'contains' as const, value: vip.address },
      })),
    },
    unreadCount: 0,
    children: [],
    depth: 0,
  }
}

/**
 * Flagged, with one child per colour. docs/01 §8.
 *
 * Children are only offered when a colour has been renamed or used, because seven identical
 * children under every Flagged row is noise in a sidebar that is meant to be quiet.
 */
function flaggedNode(flagNames: FlagName[]): SidebarNode {
  return {
    id: 'flagged',
    label: 'Flagged',
    icon: Flag,
    mailboxIds: [],
    predicate: flaggedPredicate(),
    unreadCount: 0,
    children: flagNames.map((flag) => ({
      id: `flag-${flag.color}`,
      label: flag.name,
      icon: Flag,
      flagColor: flag.color,
      mailboxIds: [],
      predicate: colourPredicate(flag.color),
      unreadCount: 0,
      children: [],
      depth: 1,
    })),
    depth: 0,
  }
}

/** The roles every account has one of, whose names say nothing about which account. */
const SHARED_ROLES = new Set(['inbox', 'drafts', 'sent', 'junk', 'trash', 'archive', 'all'])

/** The rows every sidebar's Favourites starts with, in the order a new store has them. */
const BUILTIN_ORDER: BuiltinFavourite[] = ['allInboxes', 'vips', 'flagged', 'allDrafts', 'allSent']

/**
 * Favourites in the order to draw them.
 *
 * The store's order once it has been read. Before that — the first frame, or a store too old to
 * have one — the order a new store starts with, with the mailboxes added after the built-in rows.
 * Those entries carry an id of 0, which is how the sidebar knows not to offer a drag it could
 * not report.
 */
function orderedFavourites(mailboxes: MailboxRow[], favourites: FavouriteRow[]): FavouriteRow[] {
  if (favourites.length > 0) return favourites

  const added = mailboxes
    .filter((mailbox) => mailbox.favouriteOrder !== null)
    .sort((a, b) => (a.favouriteOrder ?? 0) - (b.favouriteOrder ?? 0))
    .map((mailbox) => ({ id: 0, builtin: null, mailboxId: mailbox.id }))

  return [...BUILTIN_ORDER.map((builtin) => ({ id: 0, builtin, mailboxId: null })), ...added]
}

/**
 * Favourites: the rows every sidebar starts with and the mailboxes the user added, in the order
 * the user left them. docs/01 §3 — "reorderable by drag".
 *
 * A mailbox is labelled with its account where the name alone would not say which mailbox it is.
 * Every account has an Inbox, so an account's Inbox is always "Inbox – Google"; a folder the user
 * made is just its name, unless two favourites share it.
 */
function favouritesNodes(
  accounts: AccountRow[],
  mailboxes: MailboxRow[],
  flagNames: FlagName[],
  vips: Vip[],
  favourites: FavouriteRow[],
): SidebarNode[] {
  const order = orderedFavourites(mailboxes, favourites)
  const chosen = order
    .map((entry) => mailboxes.find((mailbox) => mailbox.id === entry.mailboxId))
    .filter((mailbox) => mailbox !== undefined)

  const builtin = (key: BuiltinFavourite): SidebarNode | null => {
    switch (key) {
      case 'allInboxes':
        return unifiedNode(mailboxes, accounts, 'all-inboxes', 'All Inboxes', 'inbox')
      case 'vips':
        return vipNode(vips)
      case 'flagged':
        return flaggedNode(flagNames)
      case 'allDrafts':
        return unifiedNode(mailboxes, accounts, 'all-drafts', 'All Drafts', 'drafts')
      case 'allSent':
        return unifiedNode(mailboxes, accounts, 'all-sent', 'All Sent', 'sent')
    }
  }

  const added = (mailbox: MailboxRow): SidebarNode => {
    const account = accounts.find((entry) => entry.id === mailbox.accountId)
    const ambiguous =
      (mailbox.role !== null && SHARED_ROLES.has(mailbox.role)) ||
      chosen.some((other) => other.id !== mailbox.id && other.displayName === mailbox.displayName)

    return {
      id: `favourite-${String(mailbox.id)}`,
      label:
        ambiguous && account !== undefined
          ? `${mailbox.displayName} – ${account.displayName}`
          : mailbox.displayName,
      icon: iconFor(mailbox.role),
      mailboxIds: [mailbox.id],
      accountId: mailbox.accountId,
      ...(account?.color == null ? {} : { accountColor: account.color }),
      unreadCount: mailbox.unreadCount,
      children: [],
      depth: 0,
    }
  }

  return order.flatMap((entry) => {
    let node: SidebarNode | null = null

    if (entry.builtin !== null) {
      node = builtin(entry.builtin)
    } else {
      const mailbox = mailboxes.find((each) => each.id === entry.mailboxId)
      // A favourite whose mailbox the tree has not caught up with yet.
      if (mailbox !== undefined) node = added(mailbox)
    }

    // Spread rather than set to `undefined`, for `exactOptionalPropertyTypes`.
    return node === null ? [] : [{ ...node, ...(entry.id > 0 ? { favouriteId: entry.id } : {}) }]
  })
}

/**
 * One account's mailboxes, as a tree.
 *
 * The folders the account files into come first, in Mail's order and at the top whatever their
 * path, because that is where Mail puts them; their own subfolders nest under them. Everything
 * else nests under the mailbox it is inside (`MailboxRow.parentId`), alphabetically at each level
 * — the one place in the sidebar where alphabetical is right, because the user made these and
 * there is no other order to respect. A mailbox whose parent is not shown — Gmail's All Mail —
 * sits at the top.
 */
function accountNodes(account: AccountRow, owned: MailboxRow[]): SidebarNode[] {
  const shown = owned.filter(
    (mailbox) => mailbox.role === null || ACCOUNT_ROLE_ORDER.includes(mailbox.role),
  )
  const shownIds = new Set(shown.map((mailbox) => mailbox.id))
  const alphabetical = (a: MailboxRow, b: MailboxRow) => a.displayName.localeCompare(b.displayName)

  const roles = ACCOUNT_ROLE_ORDER.map((role) =>
    shown.find((mailbox) => mailbox.role === role),
  ).filter((mailbox) => mailbox !== undefined)
  const roots = shown
    .filter(
      (mailbox) =>
        mailbox.role === null && (mailbox.parentId === null || !shownIds.has(mailbox.parentId)),
    )
    .sort(alphabetical)

  // Guards against a tree that loops, which paths cannot make but a bad row could.
  const placed = new Set<number>()

  const node = (mailbox: MailboxRow, depth: number): SidebarNode => {
    placed.add(mailbox.id)

    const children = shown
      .filter(
        (child) => child.role === null && child.parentId === mailbox.id && !placed.has(child.id),
      )
      .sort(alphabetical)

    return {
      // Derived from the mailbox id, which is what lets the shell open on a specific account's
      // inbox rather than on the unified row above it.
      id: `mailbox-${String(mailbox.id)}`,
      label: mailbox.displayName,
      icon: iconFor(mailbox.role),
      mailboxIds: [mailbox.id],
      accountId: mailbox.accountId,
      ...(account.color == null ? {} : { accountColor: account.color }),
      unreadCount: mailbox.unreadCount,
      depth,
      children: children.map((child) => node(child, depth + 1)),
    }
  }

  return [...roles, ...roots].map((mailbox) => node(mailbox, 0))
}

/**
 * The store payload for selecting a row, or `null` when the row is not selectable.
 *
 * Extracted from the sidebar's own click handler so that Ctrl+1-9 cannot drift away from what
 * clicking does. A row is selectable if it names mailboxes **or** carries a predicate -- the
 * type's comment says a row has "one or the other, never both", and a guard that tested only
 * the first left every predicate row dead on click.
 */
export function selectionForNode(node: SidebarNode): MailboxSelection | null {
  if (node.mailboxIds.length === 0 && node.predicate === undefined) return null

  return {
    nodeId: node.id,
    label: node.label,
    mailboxIds: node.mailboxIds,
    // Spread rather than set to `undefined`: with `exactOptionalPropertyTypes` an explicit
    // undefined is not the same as an absent key, and the list branches on absence.
    ...(node.predicate === undefined ? {} : { predicate: node.predicate }),
  }
}

export function buildSidebar(
  accounts: AccountRow[],
  mailboxes: MailboxRow[],
  smart: SmartMailbox[] = [],
  flagNames: FlagName[] = [],
  vips: Vip[] = [],
  favourites: FavouriteRow[] = [],
): SidebarSection[] {
  const favouritesSection: SidebarSection = {
    id: 'favourites',
    title: 'Favourites',
    nodes: favouritesNodes(accounts, mailboxes, flagNames, vips, favourites),
  }

  const accountSections: SidebarSection[] = accounts.map((account) => ({
    id: `account-${String(account.id)}`,
    title: account.displayName,
    nodes: accountNodes(
      account,
      mailboxes.filter((mailbox) => mailbox.accountId === account.id),
    ),
  }))

  const smartSection: SidebarSection = {
    id: 'smart',
    title: 'Smart Mailboxes',
    nodes: smart.map((box) => ({
      id: `smart-${String(box.id)}`,
      label: box.name,
      icon: Star,
      mailboxIds: [],
      predicate: box.predicate,
      // Deliberately not counted. An unread badge on a smart mailbox means running its
      // predicate on every sidebar render, and a sidebar that stalls on a five-condition
      // search over 50,000 messages is worse than one without a number on it.
      unreadCount: 0,
      children: [],
      depth: 0,
    })),
  }

  return [favouritesSection, smartSection, ...accountSections]
}

/**
 * Every row in the tree, collapsed or not, children included.
 *
 * For finding a row by id. Looking only at `section.nodes` finds the top level and nothing
 * else — which is how right-clicking an account under All Inboxes, the very row Mail's own
 * menu is shown on, opened no menu at all.
 */
export function allNodes(sections: SidebarSection[]): SidebarNode[] {
  const walk = (nodes: SidebarNode[]): SidebarNode[] =>
    nodes.flatMap((node) => [node, ...walk(node.children)])

  return walk(sections.flatMap((section) => section.nodes))
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

/**
 * Whether a row can carry the mailbox context menu.
 *
 * Exactly one real mailbox in a known account — the same predicate the drop targets use, and
 * for the same reason. Every row of that menu needs either a single mailbox or a single
 * account, and containers, unified rows, Flagged and its colours, VIPs and smart mailboxes
 * have neither.
 */
export function canOpenMailboxMenu(node: SidebarNode): boolean {
  return node.mailboxIds.length === 1 && node.accountId !== undefined
}
