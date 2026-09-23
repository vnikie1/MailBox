import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'

import { useMailboxes } from '@/app/queries'
import { useAccountsDetail } from '@/features/accounts/queries'
import type { AccountDetail } from '@/lib/generated/AccountDetail'
import type { MailboxRow } from '@/lib/generated/MailboxRow'
import {
  exportPickFolder,
  exportRun,
  mailboxMarkRead,
  mailboxRebuild,
  mailboxSetFavourite,
  mailboxUseAs,
  onMailboxRebuilt,
  onMailboxRefused,
  onTransferProgress,
  reasonFor,
  settingsOpen,
  syncNow,
  type EraseTarget,
} from '@/lib/ipc'
import { useToast } from '@/ui'

import { AccountInfoSheet } from './AccountInfoSheet'
import { MailboxContextMenu, type MailboxMenuActions } from './MailboxContextMenu'
import {
  DeleteMailboxSheet,
  EraseSheet,
  NewMailboxSheet,
  RenameMailboxSheet,
  type MailboxLocation,
} from './MailboxSheets'
import { buildSidebar, type SidebarNode } from './model'

/**
 * Whether an account's folders are Gmail labels.
 *
 * By provider, or by server for a Google address added through "Other" with an app password —
 * the same two tests the attachment check uses, because both are about how Gmail behaves rather
 * than about how the account was added.
 */
export function isGmail(account: AccountDetail | undefined): boolean {
  if (account === undefined) return false
  if (account.provider === 'google' || account.provider === 'gmail') return true

  const host = account.imap?.host.toLowerCase().replace(/\.$/, '') ?? ''
  return host === 'imap.gmail.com' || host === 'imap.googlemail.com'
}

/**
 * One level of depth in the New Mailbox popup: two figure spaces, which a native popup keeps
 * where it would collapse ordinary ones.
 */
const INDENT = '\u2007\u2007'

/**
 * Where New Mailbox can put a mailbox: each account, and inside it every mailbox that can hold
 * others.
 *
 * Walked from the sidebar's own tree (`buildSidebar`), so the popup lists them in the order and
 * at the depth the sidebar shows them and cannot drift from it. A mailbox inside one that cannot
 * hold others — or one the sidebar does not draw — is still offered, where the sidebar has it.
 */
export function mailboxLocations(
  accounts: AccountDetail[],
  mailboxes: MailboxRow[],
): MailboxLocation[] {
  const sections = buildSidebar(
    accounts.map(({ id, displayName, email, provider, color }) => ({
      id,
      displayName,
      email,
      provider,
      color,
    })),
    mailboxes,
  )

  return accounts.flatMap((account) => {
    const own = mailboxes.filter((entry) => entry.accountId === account.id)
    const known =
      own.find((entry) => entry.role === 'inbox' && entry.delimiter !== null) ??
      own.find((entry) => entry.delimiter !== null)

    const top: MailboxLocation = {
      key: `account-${String(account.id)}`,
      accountId: account.id,
      parentId: null,
      name: account.displayName,
      label: account.displayName,
      depth: 0,
      delimiter: known?.delimiter ?? null,
    }

    const walk = (nodes: SidebarNode[]): MailboxLocation[] =>
      nodes.flatMap((node) => {
        const mailbox = own.find((entry) => entry.id === node.mailboxIds[0])
        const here: MailboxLocation[] =
          mailbox?.canContain === true
            ? [
                {
                  key: `mailbox-${String(mailbox.id)}`,
                  accountId: account.id,
                  parentId: mailbox.id,
                  name: mailbox.displayName,
                  // Indented: a native popup has no other way to show depth.
                  label: `${INDENT.repeat(node.depth + 1)}${mailbox.displayName}`,
                  depth: node.depth + 1,
                  delimiter: mailbox.delimiter,
                },
              ]
            : []
        return [...here, ...walk(node.children)]
      })

    const section = sections.find((entry) => entry.id === `account-${String(account.id)}`)
    return [top, ...walk(section?.nodes ?? [])]
  })
}

export interface MailboxMenu {
  /** The menu for one sidebar row, or null for a row that has none. */
  menu: (node: SidebarNode) => ReactNode
  /** The sheets the menu opens. Rendered once, by the shell. */
  sheets: ReactNode
}

/**
 * Everything behind the mailbox context menu: what each row does, and the sheets four of them
 * open first.
 *
 * It lived in the shell while the menu had five rows, all of them one-liners. With a sheet per
 * question it would have been a hundred lines of state in a component whose job is layout.
 *
 * Each sheet's subject is kept after it closes and only its open flag is cleared, because a
 * sheet animates out: clearing the subject with the flag rewrote "Delete “Clients”?" to
 * "Delete “”?" while the user was still watching it go.
 */
export function useMailboxMenu(): MailboxMenu {
  const toast = useToast()
  const { data: mailboxes = [] } = useMailboxes()
  const { data: accounts = [] } = useAccountsDetail()

  const [createOpen, setCreateOpen] = useState(false)
  const [createIn, setCreateIn] = useState<string | null>(null)
  const [renameOpen, setRenameOpen] = useState(false)
  const [renaming, setRenaming] = useState<MailboxRow | null>(null)
  const [deleteOpen, setDeleteOpen] = useState(false)
  const [deleting, setDeleting] = useState<MailboxRow | null>(null)
  const [eraseOpen, setEraseOpen] = useState(false)
  const [erasing, setErasing] = useState<{ accountId: number; target: EraseTarget } | null>(null)
  const [infoOpen, setInfoOpen] = useState(false)
  const [infoFor, setInfoFor] = useState<number | null>(null)
  // What each rebuild was called when it was asked for, for the sentence when it finishes. A ref:
  // nothing is drawn from it.
  const rebuilding = useRef(new Map<number, string>())

  const failed = useCallback(
    (title: string) => (cause: unknown) => {
      // `reasonFor`, not `instanceof Error`: the core rejects with a plain `{ code, message }`,
      // which the reflexive idiom renders as "[object Object]".
      toast.show({ title, description: reasonFor(cause) })
    },
    [toast],
  )

  // A change the server turned down, after the core has put the sidebar back. Said here because
  // the sheet that asked for it closed long ago — the refusal arrives at the next sync.
  useEffect(() => {
    let cancelled = false
    let stop: (() => void) | undefined

    void onMailboxRefused((refused) => {
      toast.show({ title: 'The server refused a mailbox change', description: refused.message })
    }).then((unlisten) => {
      if (cancelled) unlisten()
      else stop = unlisten
    })

    return () => {
      cancelled = true
      stop?.()
    }
  }, [toast])

  // A rebuild ends in a sync, often minutes after the menu closed; this is where it says so.
  useEffect(() => {
    let cancelled = false
    let stop: (() => void) | undefined

    void onMailboxRebuilt((rebuilt) => {
      const name = rebuilding.current.get(rebuilt.mailboxId)
      rebuilding.current.delete(rebuilt.mailboxId)

      toast.show({
        title: name === undefined ? 'Mailbox rebuilt' : `Rebuilt “${name}”`,
        description:
          rebuilt.messages === 1
            ? '1 message was read again from the server.'
            : `${rebuilt.messages.toLocaleString()} messages were read again from the server.`,
      })
    }).then((unlisten) => {
      if (cancelled) unlisten()
      else stop = unlisten
    })

    return () => {
      cancelled = true
      stop?.()
    }
  }, [toast])

  const actions = useMemo<MailboxMenuActions>(
    () => ({
      // Opened on the mailbox the menu was opened on, when it can hold another; otherwise on its
      // account — the same default Mail's sheet has.
      newMailbox: (mailbox) => {
        setCreateIn(
          mailbox.canContain
            ? `mailbox-${String(mailbox.id)}`
            : `account-${String(mailbox.accountId)}`,
        )
        setCreateOpen(true)
      },

      renameMailbox: (mailbox) => {
        setRenaming(mailbox)
        setRenameOpen(true)
      },

      deleteMailbox: (mailbox) => {
        setDeleting(mailbox)
        setDeleteOpen(true)
      },

      useAs: (mailbox, usage) => {
        mailboxUseAs(mailbox.id, usage).catch(
          failed(`“${mailbox.displayName}” could not be used for that`),
        )
      },

      setFavourite: (mailbox, favourite) => {
        mailboxSetFavourite(mailbox.id, favourite).catch(
          failed(
            favourite
              ? `“${mailbox.displayName}” could not be added to Favourites`
              : `“${mailbox.displayName}” could not be removed from Favourites`,
          ),
        )
      },

      exportMailbox: (mailboxId, label) => {
        void (async () => {
          const directory = await exportPickFolder()
          if (directory === null) return

          // Export returns as soon as the work is *scheduled*, so without listening for the
          // finish this row would appear to do nothing at all. `finished` is the completion
          // signal — `done` is a running count, and treating it as one fired on the first
          // message.
          let stop: (() => void) | null = null
          stop = await onTransferProgress((progress) => {
            const result = progress.finished
            if (result === null) return

            toast.show({
              title:
                result.error === null ? `Exported “${label}”` : `“${label}” could not be exported`,
              description:
                result.error ?? `${String(result.messages)} messages written to ${directory}.`,
            })

            // This listener exists for one export, and would otherwise fire again for every
            // later transfer, imports included.
            stop?.()
          })

          await exportRun([mailboxId], 'mbox', directory)
        })().catch(failed('That mailbox could not be exported'))
      },

      // Said at once, because the work happens in a sync that may take minutes on a large
      // mailbox; the finish is said by the listener above.
      rebuild: (mailbox, label) => {
        rebuilding.current.set(mailbox.id, label)
        mailboxRebuild(mailbox.id)
          .then(() => {
            toast.show({
              title: `Rebuilding “${label}”`,
              description: 'Every message in it is being read again from the server.',
            })
          })
          .catch(failed(`“${label}” could not be rebuilt`))
      },

      eraseDeleted: (accountId) => {
        setErasing({ accountId, target: 'trash' })
        setEraseOpen(true)
      },

      eraseJunk: (accountId) => {
        setErasing({ accountId, target: 'junk' })
        setEraseOpen(true)
      },

      markAllRead: (mailboxId) => {
        mailboxMarkRead(mailboxId)
          .then((changed) => {
            toast.show({
              title:
                changed === 0
                  ? 'Nothing was unread'
                  : changed === 1
                    ? 'Marked 1 message as read'
                    : `Marked ${String(changed)} messages as read`,
            })
          })
          .catch(failed('Those messages could not be marked as read'))
      },

      synchronise: (accountId) => {
        syncNow(accountId).catch(failed('That account could not be synchronised'))
      },

      // Opens Settings on that account rather than on the pane: with several accounts, "the
      // Accounts pane" left the user to find the one they had just named.
      editAccount: (accountId) => {
        settingsOpen('accounts', accountId).catch(failed('Settings could not be opened'))
      },

      accountInfo: (accountId) => {
        setInfoFor(accountId)
        setInfoOpen(true)
      },
    }),
    [failed, toast],
  )

  const menu = useCallback(
    (node: SidebarNode): ReactNode => {
      const mailbox = mailboxes.find((entry) => entry.id === node.mailboxIds[0])
      if (mailbox === undefined) return null

      const account = accounts.find((entry) => entry.id === mailbox.accountId)
      const own = mailboxes.filter((entry) => entry.accountId === mailbox.accountId)
      const hasServer = account !== undefined && account.imap !== null

      return (
        <MailboxContextMenu
          node={node}
          mailbox={mailbox}
          accountName={account?.displayName ?? 'this account'}
          canSync={hasServer && account.syncEnabled}
          canChooseRole={hasServer && !isGmail(account) && mailbox.role !== 'inbox'}
          trash={own.find((entry) => entry.role === 'trash')}
          junk={own.find((entry) => entry.role === 'junk')}
          actions={actions}
        />
      )
    },
    [mailboxes, accounts, actions],
  )

  const locations = useMemo(() => mailboxLocations(accounts, mailboxes), [accounts, mailboxes])

  const deletingAccount = accounts.find((entry) => entry.id === deleting?.accountId)
  const erasingAccount = accounts.find((entry) => entry.id === erasing?.accountId)
  const erasingMailbox =
    erasing === null
      ? undefined
      : mailboxes.find(
          (entry) => entry.accountId === erasing.accountId && entry.role === erasing.target,
        )

  const sheets = (
    <>
      <NewMailboxSheet
        open={createOpen}
        onOpenChange={setCreateOpen}
        locations={locations}
        initial={createIn}
      />
      <RenameMailboxSheet open={renameOpen} onOpenChange={setRenameOpen} mailbox={renaming} />
      <DeleteMailboxSheet
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        mailbox={deleting}
        gmail={isGmail(deletingAccount)}
        hasServer={deletingAccount !== undefined && deletingAccount.imap !== null}
      />
      <EraseSheet
        open={eraseOpen}
        onOpenChange={setEraseOpen}
        target={erasing?.target ?? 'trash'}
        accountId={erasing?.accountId ?? null}
        accountName={erasingAccount?.displayName ?? ''}
        mailbox={erasingMailbox}
        hasServer={erasingAccount !== undefined && erasingAccount.imap !== null}
      />
      <AccountInfoSheet
        open={infoOpen}
        onOpenChange={setInfoOpen}
        account={accounts.find((entry) => entry.id === infoFor)}
      />
    </>
  )

  return { menu, sheets }
}
