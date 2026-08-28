import { useCallback, useEffect, useRef, useState } from 'react'
import { ChevronLeft } from 'lucide-react'

import { cx } from '@/lib/cx'
import { LIST_MAX, LIST_MIN, SIDEBAR_MAX, SIDEBAR_MIN, useLayoutStore } from '@/store/layout'
import {
  useAccounts,
  useArchiveMessages,
  useDeleteMessages,
  useMailboxes,
  useMoveMessages,
  useToggleFlag,
  useToggleRead,
} from '@/app/queries'
import { useMailStore } from '@/store/mail'
import { Button, useToast } from '@/ui'
import { MessageList } from '@/features/messageList/MessageList'
import { Reader } from '@/features/reader/Reader'
import { Sidebar } from '@/features/sidebar/Sidebar'
import { MailboxPicker, useUndo } from '@/features/organise'
import { useShortcuts } from '@/app/useShortcuts'
import { ShortcutSheet } from '@/features/help/ShortcutSheet'
import { ScopeBar, useSearch } from '@/features/search'
import { SaveSearchSheet } from '@/features/search/SaveSearchSheet'
import { junkMark, rulesRun } from '@/lib/organise'
import { composeBlank, composeOpen, syncAll } from '@/lib/ipc'

import { PaneDivider } from './PaneDivider'
import { useBreakpoint } from './useBreakpoint'

import styles from './AppShell.module.css'

/** Which pane is on screen in the one-pane layout. docs/01 §1 — push navigation. */
type Level = 'mailboxes' | 'list' | 'reader'

/**
 * The window. docs/01 §1.
 *
 * Three resizable columns above 1000px, two between 1000 and 700, and one with push
 * navigation below that. docs/01 §1 singles the breakpoints out as "where every Windows
 * client falls apart", so they are treated as a feature rather than as a media query
 * bolted on at the end.
 *
 * There is no separate toolbar band. docs/02 §6.1 describes one unified 52pt bar across
 * the window, but the macOS 26 captures in assets/reference/ show three pane headers at a
 * shared height instead — sidebar toggle over the sidebar, mailbox title over the list,
 * actions and search over the reader. Stacking a window-wide toolbar on top of those, as
 * the first version of this did, cost 104pt of chrome against Mail's 52.
 */
export interface AppShellProps {
  onOpenSettings?: (() => void) | undefined
}

export function AppShell({ onOpenSettings }: AppShellProps) {
  const breakpoint = useBreakpoint()

  const sidebarWidth = useLayoutStore((state) => state.sidebarWidth)
  const listWidth = useLayoutStore((state) => state.listWidth)
  const sidebarCollapsed = useLayoutStore((state) => state.sidebarCollapsed)
  const classicLayout = useLayoutStore((state) => state.classicLayout)
  const setSidebarWidth = useLayoutStore((state) => state.setSidebarWidth)
  const setListWidth = useLayoutStore((state) => state.setListWidth)
  const toggleSidebar = useLayoutStore((state) => state.toggleSidebar)

  const selectedMessageIds = useMailStore((state) => state.selectedMessageIds)
  const selectedNodeId = useMailStore((state) => state.selection.nodeId)
  const selectionMailboxIds = useMailStore((state) => state.selection.mailboxIds)
  const selectionLabel = useMailStore((state) => state.selection.label)
  const selectMailbox = useMailStore((state) => state.selectMailbox)

  const { data: mailboxes = [] } = useMailboxes()
  const { data: accounts = [] } = useAccounts()
  const move = useMoveMessages()
  const remove = useDeleteMessages()
  const archive = useArchiveMessages()
  const toggleRead = useToggleRead()
  const toggleFlag = useToggleFlag()
  const toast = useToast()

  const [movingTo, setMovingTo] = useState(false)
  const [savingSearch, setSavingSearch] = useState(false)
  const [showingShortcuts, setShowingShortcuts] = useState(false)

  // The one selected message, when there is exactly one. Reply and forward act on a single
  // message; with several selected there is no message to reply to.
  const only = selectedMessageIds.length === 1 ? selectedMessageIds[0] : undefined

  const search = useSearch(selectionMailboxIds)
  const searching = search.text.trim() !== ''

  // Ctrl+Z and Ctrl+Shift+Z. Owned by the hook because undo has to reverse a *database*
  // change, and the stack lives in the core rather than in React.
  const { undo, redo } = useUndo()

  const failed = useCallback(
    (title: string) => (error: unknown) => {
      toast.show({
        title,
        description: error instanceof Error ? error.message : String(error),
      })
    },
    [toast],
  )

  // One dispatcher for the whole application. See `app/shortcuts.ts` for why they are not
  // registered where they are used.
  useShortcuts(
    {
      newMessage: useCallback(() => {
        // The account whose mailbox is open, falling back to the first. Composing from an
        // arbitrary account would put the wrong address in the From line.
        const account =
          mailboxes.find((mailbox) => selectionMailboxIds.includes(mailbox.id))?.accountId ??
          accounts[0]?.id

        if (account === undefined) return
        void composeBlank(account).catch(failed('A new message could not be opened'))
      }, [mailboxes, selectionMailboxIds, accounts, failed]),

      reply: useCallback(() => {
        if (only !== undefined) void composeOpen(only, 'reply').catch(failed('Reply failed'))
      }, [only, failed]),
      replyAll: useCallback(() => {
        if (only !== undefined) void composeOpen(only, 'replyAll').catch(failed('Reply failed'))
      }, [only, failed]),
      forward: useCallback(() => {
        if (only !== undefined) void composeOpen(only, 'forward').catch(failed('Forward failed'))
      }, [only, failed]),

      // Each message goes to its *own* account's Archive, resolved in the core. "All Inboxes"
      // means a selection routinely spans two accounts, and there is no single archive.
      archive: useCallback(() => {
        archive.mutate(selectedMessageIds)
      }, [selectedMessageIds, archive]),

      delete: useCallback(() => {
        remove.mutate({ ids: selectedMessageIds, permanent: false })
      }, [selectedMessageIds, remove]),

      deletePermanently: useCallback(() => {
        remove.mutate({ ids: selectedMessageIds, permanent: true })
      }, [selectedMessageIds, remove]),

      // The core decides the direction from the stored rows, so the window keeps no second copy
      // of what is read — which would be stale the moment another client changed it.
      toggleRead: useCallback(() => {
        toggleRead.mutate(selectedMessageIds)
      }, [selectedMessageIds, toggleRead]),

      flag: useCallback(() => {
        toggleFlag.mutate(selectedMessageIds)
      }, [selectedMessageIds, toggleFlag]),

      markJunk: useCallback(() => {
        void junkMark(selectedMessageIds, true).catch(failed('That could not be marked as junk'))
      }, [selectedMessageIds, failed]),

      moveTo: useCallback(() => {
        setMovingTo(true)
      }, []),

      runRules: useCallback(() => {
        void rulesRun(selectedMessageIds)
          .then((report) => {
            toast.show({
              title:
                report.matched === 0
                  ? 'No rules matched'
                  : `${String(report.matched)} of ${String(report.examined)} messages matched`,
            })
          })
          .catch(failed('The rules could not be run'))
      }, [selectedMessageIds, toast, failed]),

      undo,
      redo,

      search: useCallback(() => {
        // Focus rather than a mode: the field is always there, and Ctrl+F should put the caret
        // in it exactly as it would in any other application.
        document
          .querySelector<HTMLInputElement>('input[type="text"][placeholder="Search"]')
          ?.focus()
      }, []),

      getMail: useCallback(() => {
        void syncAll().catch(failed('Could not check for new mail'))
      }, [failed]),

      toggleSidebar: useCallback(() => {
        toggleSidebar()
      }, [toggleSidebar]),

      showShortcuts: useCallback(() => {
        setShowingShortcuts(true)
      }, []),
    },
    { hasSelection: selectedMessageIds.length > 0 },
  )

  // Open on the first account's inbox once the mailboxes arrive. The store starts with no
  // selection because it no longer owns the data and cannot know what exists.
  const firstInbox = mailboxes.find((mailbox) => mailbox.role === 'inbox') ?? mailboxes[0]
  useEffect(() => {
    if (selectedNodeId === '' && firstInbox) {
      selectMailbox({
        nodeId: `mailbox-${String(firstInbox.id)}`,
        label: firstInbox.displayName,
        mailboxIds: [firstInbox.id],
      })
    }
  }, [selectedNodeId, firstInbox, selectMailbox])

  const [level, setLevel] = useState<Level>('list')

  /**
   * Push navigation in the one-pane layout. docs/01 §1.
   *
   * These watch for the selection *changing*, not for it being non-empty. A thread is
   * already selected at startup, so an effect that pushed whenever one exists would land
   * on the reader the instant the window narrowed — skipping past the list the user was
   * looking at. The refs remember what was last seen so the first run after a resize is a
   * no-op rather than a jump.
   */
  const lastThread = useRef(selectedMessageIds[0])
  const lastMailbox = useRef(selectedNodeId)

  useEffect(() => {
    const current = selectedMessageIds[0]
    const changed = current !== lastThread.current
    lastThread.current = current

    if (changed && breakpoint === 'one' && selectedMessageIds.length === 1) setLevel('reader')
  }, [breakpoint, selectedMessageIds])

  useEffect(() => {
    const changed = selectedNodeId !== lastMailbox.current
    lastMailbox.current = selectedNodeId

    if (changed && breakpoint === 'one') setLevel('list')
  }, [breakpoint, selectedNodeId])

  // Narrowing to one pane always lands on the list, whatever was selected before.
  useEffect(() => {
    if (breakpoint === 'one') setLevel('list')
  }, [breakpoint])

  const showSidebar = breakpoint === 'three' && !sidebarCollapsed
  const showList = breakpoint !== 'one' || level === 'list'
  const showReader = breakpoint !== 'one' || level === 'reader'

  return (
    <div className={styles.window}>
      <div className={cx(styles.body, classicLayout && styles.classic)}>
        {breakpoint === 'one' && level !== 'mailboxes' && (
          <div className={styles.backBar}>
            <Button
              variant="plain"
              icon={ChevronLeft}
              onClick={() => {
                setLevel(level === 'reader' ? 'list' : 'mailboxes')
              }}
            >
              {level === 'reader' ? 'Messages' : 'Mailboxes'}
            </Button>
          </div>
        )}

        {(showSidebar || (breakpoint === 'one' && level === 'mailboxes')) && (
          <>
            <div
              className={styles.sidebarPane}
              style={breakpoint === 'one' ? undefined : { width: `${String(sidebarWidth)}px` }}
            >
              <Sidebar onOpenSettings={onOpenSettings} />
            </div>
            {breakpoint === 'three' && (
              <PaneDivider
                label="Sidebar width"
                value={sidebarWidth}
                min={SIDEBAR_MIN}
                max={SIDEBAR_MAX}
                onChange={setSidebarWidth}
              />
            )}
          </>
        )}

        {showList && (
          <div
            className={styles.listPane}
            style={
              breakpoint === 'one' || classicLayout
                ? undefined
                : { width: `${String(listWidth)}px` }
            }
          >
            <MessageList
              showSidebarToggle={!showSidebar}
              searchRows={searching ? search.visible.map((hit) => hit.row) : undefined}
              scopeBar={
                searching ? (
                  <ScopeBar
                    place={search.place}
                    state={search.state}
                    mailboxLabel={selectionLabel}
                    resultCount={search.visible.length}
                    onPlaceChange={search.setPlace}
                    onStateChange={search.setState}
                    onSaveSearch={() => {
                      setSavingSearch(true)
                    }}
                  />
                ) : undefined
              }
            />
          </div>
        )}

        {showList && showReader && breakpoint !== 'one' && !classicLayout && (
          <PaneDivider
            label="Message list width"
            value={listWidth}
            min={LIST_MIN}
            max={LIST_MAX}
            onChange={setListWidth}
          />
        )}

        {showReader && (
          <div className={styles.readerPane}>
            <Reader
              toolbar={{
                search: search.text,
                onSearchChange: search.setText,
                onSearchCommit: search.commit,
              }}
            />
          </div>
        )}
      </div>

      <ShortcutSheet open={showingShortcuts} onOpenChange={setShowingShortcuts} />

      <SaveSearchSheet open={savingSearch} onOpenChange={setSavingSearch} text={search.text} />

      <MailboxPicker
        open={movingTo}
        onOpenChange={setMovingTo}
        mailboxes={mailboxes}
        accounts={accounts}
        title={
          selectedMessageIds.length === 1
            ? 'Move message to…'
            : `Move ${String(selectedMessageIds.length)} messages to…`
        }
        onChoose={(mailboxId) => {
          move.mutate({ ids: selectedMessageIds, mailboxId })
        }}
      />
    </div>
  )
}
