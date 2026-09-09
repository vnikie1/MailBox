import { useEffect, useMemo, useState, type KeyboardEvent } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'

import { useAppearanceSync } from '@/app/useAppearanceSync'
import { useAccountEvents } from '@/features/accounts'
import { AccountsSettings } from '@/features/accounts/AccountsSettings'
import { NotificationSettings } from '@/features/accounts/NotificationSettings'
import { ComposingSettings } from '@/features/compose/ComposingSettings'
import { JunkSettings } from '@/features/organise/JunkSettings'
import { OrganiseSettings } from '@/features/organise/OrganiseSettings'
import { ReadingSettings } from '@/features/reader/ReadingSettings'
import { onSettingsPane, setWindowTitle, type SettingsPane } from '@/lib/ipc'
import { ToastProvider } from '@/ui'

import { AdvancedSettings } from './AdvancedSettings'
import { AppearanceSettings } from './AppearanceSettings'
import { PrivacyStatement } from './PrivacyStatement'
import { SignatureSettings } from './SignatureSettings'
import { UpdateSettings } from './UpdateSettings'
import { PANES, paneFrom } from './panes'
import styles from './SettingsWindow.module.css'
import settings from './settings.module.css'

/**
 * Settings, in a window of its own. docs/06 Phase 11.
 *
 * ## What this replaces
 *
 * Six sections stacked in one modal sheet, reached only through the sidebar, and mounted inside
 * `AccountsGate` — so the settings a user could open were coupled to the first-run account
 * assistant, and closing one meant thinking about the other. Stacking also meant that every
 * section loaded at once: opening Settings to change the undo delay ran the junk filter's status
 * query and every account's notification preferences.
 *
 * A pane is mounted only while it is shown, so a pane costs nothing until it is opened. That is
 * not a performance argument — none of these queries is expensive — it is a correctness one: the
 * Advanced pane reads the crash-report folder from disk, and a settings window that touches the
 * filesystem whenever it opens is doing work nobody asked for.
 *
 * ## Its own providers
 *
 * A second OS window is a second React root with nothing shared but `localStorage`. It needs its
 * own QueryClient (Accounts and Signatures both use one) and its own ToastProvider. It also runs
 * `useAppearanceSync`, which matters more here than anywhere: this is the window where the theme
 * is changed, and a theme control that does not repaint the window it is in would look broken
 * before the user ever looked at the mailbox behind it.
 */
function Panes() {
  useAppearanceSync()

  // Settings is its own OS window with its own `QueryClient`, so it hears nothing the main
  // window subscribes to. `useAccountEvents` was mounted once "near the root" — the root of the
  // *main* window — and this one listened to nothing at all.
  //
  // So adding an account did not appear in the list here. The core announced it, the main
  // window acted on it, and this window went on showing the answer it had cached when it
  // opened; closing Settings and opening it again remounted the query and it appeared. Which
  // makes it look like the account was not really added.
  useAccountEvents()

  const [pane, setPane] = useState<SettingsPane>(() =>
    paneFrom(new URLSearchParams(window.location.search).get('pane')),
  )

  // Reopening Settings while it is already open moves it to the pane that was asked for rather
  // than opening a second window. See `settings_open` in ipc/window.rs.
  useEffect(() => {
    let cancelled = false
    let stop: (() => void) | undefined

    void onSettingsPane(setPane).then((unlisten) => {
      if (cancelled) unlisten()
      else stop = unlisten
    })

    return () => {
      cancelled = true
      stop?.()
    }
  }, [])

  useEffect(() => {
    const label = PANES.find((entry) => entry.id === pane)?.label
    const title = label === undefined ? 'Settings' : `${label} — Settings`

    // Both, and they are not the same thing. `document.title` is the WebView's title and is
    // what the browser path shows; the OS window keeps whatever the builder gave it until it
    // is told otherwise. Setting only the first looked right in Playwright and left the real
    // window reading "Settings" for ever — checked by walking the UIA tree, where the window's
    // Name is the OS title.
    document.title = title
    void setWindowTitle(title)
  }, [pane])

  const current = PANES.find((entry) => entry.id === pane)

  /**
   * Up and Down walk the pane list, Home and End jump to its ends.
   *
   * A settings sidebar that can only be operated by clicking is a settings sidebar half the
   * people who need it cannot reach. Both Windows' own Settings and Mail's pane list move the
   * selection with the arrows, and focus follows — so the pane changes as you walk it, which
   * is what makes walking it useful.
   */
  const walk = (event: KeyboardEvent<HTMLElement>) => {
    const keys = ['ArrowDown', 'ArrowUp', 'Home', 'End']
    if (!keys.includes(event.key)) return

    const index = PANES.findIndex((entry) => entry.id === pane)
    const last = PANES.length - 1

    const next =
      event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? last
          : event.key === 'ArrowUp'
            ? (index + last) % PANES.length
            : (index + 1) % PANES.length

    const target = PANES[next]
    if (target === undefined) return

    event.preventDefault()
    setPane(target.id)

    // Held in a local, because `currentTarget` is only meaningful while the event is being
    // dispatched — React clears it before the frame this runs in.
    const list = event.currentTarget

    // Read back from the DOM after React has committed, rather than kept in an array of refs:
    // the entry that has just become current is the one carrying `aria-current`.
    requestAnimationFrame(() => {
      list.querySelector<HTMLButtonElement>('[aria-current="true"]')?.focus()
    })
  }

  return (
    <div className={styles.window}>
      <nav className={styles.nav} aria-label="Settings" onKeyDown={walk}>
        {PANES.map((entry) => {
          const Icon = entry.icon
          const chosen = pane === entry.id

          return (
            <button
              key={entry.id}
              type="button"
              className={styles.navItem}
              // `aria-current` rather than `aria-selected`: these are navigation, not a
              // listbox, and a screen reader announces "current page" — which is what they
              // are — instead of an option in a set the user is choosing between.
              aria-current={chosen}
              // One tab stop for the list, as a navigation list should be — seven stops to
              // walk past the sidebar on the way to the pane is seven too many.
              tabIndex={chosen ? 0 : -1}
              onClick={() => {
                setPane(entry.id)
              }}
            >
              <Icon className={styles.navIcon} aria-hidden />
              {entry.label}
            </button>
          )
        })}
      </nav>

      {/* Keyed by pane, so switching panes rebuilds the scroller rather than reconciling one
          pane's sections against the next one's. General is tall enough to scroll and the
          others are not, and a reconciled scroller keeps the scrollTop it had — which would
          leave a short pane opening part-way down itself. */}
      <main key={pane} className={styles.pane}>
        <div className={styles.content}>
          {/* The pane's name, said in the pane. It used to be in the title bar and nowhere
              else, so General opened on a heading that read "Appearance". */}
          <h1 className={settings.title}>{current?.label ?? 'Settings'}</h1>

          {pane === 'general' && (
            <>
              <AppearanceSettings />
              <NotificationSettings />
              <UpdateSettings />
            </>
          )}
          {pane === 'accounts' && <AccountsSettings />}
          {pane === 'composing' && <ComposingSettings />}
          {pane === 'signatures' && <SignatureSettings />}
          {pane === 'rules' && (
            <>
              <OrganiseSettings />
              <JunkSettings />
            </>
          )}
          {pane === 'privacy' && (
            <>
              <ReadingSettings />
              <PrivacyStatement />
            </>
          )}
          {pane === 'advanced' && <AdvancedSettings />}
        </div>
      </main>
    </div>
  )
}

export function SettingsWindow() {
  const client = useMemo(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: { staleTime: 5 * 60 * 1000, refetchOnWindowFocus: false, retry: 1 },
        },
      }),
    [],
  )

  return (
    <QueryClientProvider client={client}>
      <ToastProvider>
        <Panes />
      </ToastProvider>
    </QueryClientProvider>
  )
}
