import { useEffect, useMemo, useState } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'

import { useAppearanceSync } from '@/app/useAppearanceSync'
import { AccountsSettings } from '@/features/accounts/AccountsSettings'
import { NotificationSettings } from '@/features/accounts/NotificationSettings'
import { ComposingSettings } from '@/features/compose/ComposingSettings'
import { JunkSettings } from '@/features/organise/JunkSettings'
import { OrganiseSettings } from '@/features/organise/OrganiseSettings'
import { ReadingSettings } from '@/features/reader/ReadingSettings'
import { onSettingsPane, type SettingsPane } from '@/lib/ipc'
import { ToastProvider } from '@/ui'

import { AdvancedSettings } from './AdvancedSettings'
import { AppearanceSettings } from './AppearanceSettings'
import { PrivacyStatement } from './PrivacyStatement'
import { SignatureSettings } from './SignatureSettings'
import { PANES, paneFrom } from './panes'
import styles from './SettingsWindow.module.css'

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
    // The title carries the pane, as a Windows settings window does. It is also what the
    // taskbar and Alt-Tab show, which is the difference between one of several open windows
    // being findable and not.
    document.title = label === undefined ? 'Settings' : `${label} — Settings`
  }, [pane])

  return (
    <div className={styles.window}>
      <nav className={styles.nav} aria-label="Settings">
        {PANES.map((entry) => {
          const Icon = entry.icon
          return (
            <button
              key={entry.id}
              type="button"
              className={styles.navItem}
              // `aria-current` rather than `aria-selected`: these are navigation, not a
              // listbox, and a screen reader announces "current page" — which is what they
              // are — instead of an option in a set the user is choosing between.
              aria-current={pane === entry.id}
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

      <main className={styles.pane} aria-live="polite">
        {pane === 'general' && (
          <>
            <AppearanceSettings />
            <NotificationSettings />
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
