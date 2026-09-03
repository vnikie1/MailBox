import { useCallback, useEffect, useRef, useState } from 'react'
import { ChevronLeft } from 'lucide-react'

import { cx } from '@/lib/cx'
import { LIST_MAX, LIST_MIN, SIDEBAR_MAX, SIDEBAR_MIN, useLayoutStore } from '@/store/layout'
import {
  useAccounts,
  useArchiveMessages,
  useDeleteMessages,
  useFlagNames,
  useMailboxes,
  useMoveMessages,
  useSmartMailboxes,
  useThread,
  useToggleFlag,
  useToggleRead,
  useVips,
} from '@/app/queries'
import { useMailStore } from '@/store/mail'
import { Button, useToast } from '@/ui'
import { MessageList } from '@/features/messageList/MessageList'
import { Reader } from '@/features/reader/Reader'
import { RedirectSheet } from '@/features/reader/RedirectSheet'
import { Sidebar } from '@/features/sidebar/Sidebar'
import { buildSidebar, selectionForNode } from '@/features/sidebar/model'
import { MailboxPicker, useUndo } from '@/features/organise'
import { useShortcuts } from '@/app/useShortcuts'
import { ShortcutSheet } from '@/features/help/ShortcutSheet'
import { ScopeBar, useSearch } from '@/features/search'
import { SaveSearchSheet } from '@/features/search/SaveSearchSheet'
import { junkMark, rulesRun } from '@/lib/organise'
import { composeOpen, onJumpListTask, settingsOpen, syncAll } from '@/lib/ipc'

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
export function AppShell() {
  const breakpoint = useBreakpoint()

  /**
   * Settings is a window of its own since Phase 11, so opening it is an IPC call rather than
   * a piece of state the root has to hold. It used to be a sheet mounted by the accounts
   * gate, which meant the root threaded an open flag and two callbacks through three
   * components to reach the one button that set it.
   */
  const openSettings = useCallback(() => {
    void settingsOpen()
  }, [])

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
  const moveInThread = useMailStore((state) => state.moveInThread)

  const { data: mailboxes = [] } = useMailboxes()
  const { data: accounts = [] } = useAccounts()

  // For Ctrl+1-9 only. The sidebar mounts these same queries, so this shares its cache
  // entries rather than issuing a second set of reads.
  const { data: smart = [] } = useSmartMailboxes()
  const { data: flagNames = [] } = useFlagNames()
  const { data: vips = [] } = useVips()
  const move = useMoveMessages()
  const remove = useDeleteMessages()
  const archive = useArchiveMessages()
  const toggleRead = useToggleRead()
  const toggleFlag = useToggleFlag()
  const toast = useToast()

  const [movingTo, setMovingTo] = useState(false)
  const [redirecting, setRedirecting] = useState(false)
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

  // Shared by Ctrl+F and the taskbar's Search task, which have to do the same thing — two
  // implementations would be two answers to "what does Search mean".
  const focusSearch = useCallback(() => {
    // Focus rather than a mode: the field is always there, and Ctrl+F should put the caret
    // in it exactly as it would in any other application.
    //
    // Selected by a data attribute the field sets for this purpose. The previous selector was
    // `input[type="text"][placeholder="Search"]`, which matched nothing: TextField renders a
    // bare <input> and nothing passes `type`, so the attribute is absent and an attribute
    // selector does not see the reflected default. Ctrl+F had therefore never worked — the
    // shortcut was registered, the handler ran, and it focused nothing. Found by pressing it.
    document.querySelector<HTMLInputElement>('input[data-shortcut="search"]')?.focus()
  }, [])

  // The taskbar Jump List. New Message is handled in Rust — it opens a real window — so only
  // the two that are places in this UI arrive here.
  useEffect(() => {
    let cancelled = false
    let off: (() => void) | undefined

    void onJumpListTask((task) => {
      if (task === '--inbox') {
        // The unified row, matching what the sidebar builds. Selecting one account's inbox
        // would be a guess about which account the user meant.
        selectMailbox({
          nodeId: 'all-inboxes',
          label: 'All Inboxes',
          mailboxIds: mailboxes
            .filter((mailbox) => mailbox.role === 'inbox')
            .map((mailbox) => mailbox.id),
        })
      } else if (task === '--search') focusSearch()
    }).then((unlisten) => {
      if (cancelled) unlisten()
      else off = unlisten
    })

    return () => {
      cancelled = true
      off?.()
    }
  }, [focusSearch, selectMailbox, mailboxes])

  // One dispatcher for the whole application. See `app/shortcuts.ts` for why they are not
  // registered where they are used.
  // One set of callbacks, driving both the keyboard shortcuts and the toolbar buttons.
  //
  // They drove only the shortcuts until now. Every button in the toolbar -- New Message, Reply,
  // Reply All, Forward, Archive, Delete, Junk, Move to, Flag -- rendered, showed the right
  // enabled state, showed a tooltip, and did nothing at all, because `ToolbarProps` carried no
  // action callbacks for them to call. See the note in Toolbar.tsx.
  // Shares the reader's cache entry rather than issuing a second read: the reader mounts
  // `useThread` with the same key, so this is the same query.
  const { data: openThread = [] } = useThread(only ?? null)
  const selectedMessage = openThread.find((message) => message.id === only)

  const actions = {
    newMessage: useCallback(() => {
      // `composeOpen` opens the window. `composeBlank` -- which this called until now --
      // returns the *contents* of a blank draft and opens nothing, so New Message fetched a
      // draft, dropped it on the floor and left no trace. The compose window calls
      // `composeBlank` itself once it is open, which is what that command is for.
      void composeOpen().catch(failed('A new message could not be opened'))
    }, [failed]),

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

    // Ctrl+Shift+E. The feature existed and the shortcut did not reach it: the redirect sheet
    // is owned by each message row inside the reader, opened by a per-message button, so
    // there was nothing for a shell-level chord to call and no handler was written. The shell
    // owns a second copy for the selected message, the same way it owns the mailbox picker
    // for Move To.
    redirect: useCallback(() => {
      if (only !== undefined) setRedirecting(true)
    }, [only]),

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

    // Ctrl+1 to Ctrl+9. docs/01 §14. Jumps to the nth row of Favourites, which is the list
    // Mail's own Cmd+1-9 walks, and it is built from the same `buildSidebar` the sidebar
    // renders -- so the nth shortcut and the nth visible row cannot disagree.
    //
    // This was listed in the Help sheet and bound nowhere. `parseChord` returns null for the
    // range `Ctrl+1-9`, so the table skipped the row and no handler was ever written for it;
    // the dispatcher now special-cases the digits.
    jumpToMailbox: useCallback(
      (position: number) => {
        const favourites = buildSidebar(accounts, mailboxes, smart, flagNames, vips)[0]
        const node = favourites?.nodes[position - 1]
        if (node === undefined) return

        const selection = selectionForNode(node)
        if (selection !== null) selectMailbox(selection)
      },
      [accounts, mailboxes, smart, flagNames, vips, selectMailbox],
    ),

    // Ctrl+↓ and Ctrl+↑. docs/01 §14 — within the open conversation, not between rows, which
    // is what the plain arrows already do in the list.
    //
    // Both were listed in the Help sheet and bound nowhere: `parseChord` rejected every arrow,
    // including a modified one, so the dispatcher skipped their rows and no handler was ever
    // written. The reader had no notion of a position inside a thread either; it has one now,
    // in the store, so the chord and the pane agree on where "here" is.
    nextInThread: useCallback(() => {
      moveInThread(
        1,
        openThread.map((message) => message.id),
      )
    }, [moveInThread, openThread]),

    previousInThread: useCallback(() => {
      moveInThread(
        -1,
        openThread.map((message) => message.id),
      )
    }, [moveInThread, openThread]),

    search: focusSearch,

    getMail: useCallback(() => {
      void syncAll().catch(failed('Could not check for new mail'))
    }, [failed]),

    toggleSidebar: useCallback(() => {
      toggleSidebar()
    }, [toggleSidebar]),

    showShortcuts: useCallback(() => {
      setShowingShortcuts(true)
    }, []),

    settings: openSettings,
  }

  useShortcuts(actions, { hasSelection: selectedMessageIds.length > 0 })

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
              <Sidebar onOpenSettings={openSettings} />
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
                actions,
              }}
            />
          </div>
        )}
      </div>

      <ShortcutSheet open={showingShortcuts} onOpenChange={setShowingShortcuts} />

      <SaveSearchSheet open={savingSearch} onOpenChange={setSavingSearch} text={search.text} />

      {only !== undefined && (
        <RedirectSheet
          open={redirecting}
          onOpenChange={setRedirecting}
          messageId={only}
          subject={selectedMessage?.subject ?? ''}
        />
      )}

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
