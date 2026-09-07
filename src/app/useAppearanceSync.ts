import { useEffect } from 'react'
import type { UnlistenFn } from '@tauri-apps/api/event'

import {
  accentForeground,
  applyAppearance,
  applyWindowActive,
  resolveAccent,
  resolveTheme,
  rgbNumber,
} from '@/lib/appearance'
import {
  getAppearance,
  onAppearanceChanged,
  onDisplayPreferencesChanged,
  onWindowFocusChanged,
  setBadgePaint,
} from '@/lib/ipc'
import { useAppearanceStore } from '@/store/appearance'
import { useSettingsStore } from '@/store/settings'

/**
 * Keep the document in step with the OS appearance and the user's overrides.
 *
 * Two separate concerns, deliberately kept apart. The first effect ingests what Windows
 * reports and never touches the DOM; the core pushes changes and nothing here polls
 * (standing rule 14). The second resolves that against the user's preferences and writes
 * the result to <html>, so a settings change repaints the app by exactly the same path an
 * OS change does — there is only ever one way for the appearance to reach the document.
 *
 * Since Phase 11 the preferences live in a window of their own, so there is a third
 * subscription: the other window announcing a change. It lands in the same store the local
 * controls write to, which means a theme changed in Settings reaches the mailbox by the
 * identical path — the second effect neither knows nor cares which window it came from.
 */
export function useAppearanceSync(): void {
  const setAppearance = useAppearanceStore((state) => state.setAppearance)
  const applyRemote = useSettingsStore((state) => state.applyRemote)
  const setWindowActive = useAppearanceStore((state) => state.setWindowActive)
  const appearance = useAppearanceStore((state) => state.appearance)

  // Selected field by field rather than as an object: a fresh object every render would
  // re-run the effect below on every render.
  const theme = useSettingsStore((state) => state.theme)
  const density = useSettingsStore((state) => state.density)
  const transparency = useSettingsStore((state) => state.transparency)
  const accent = useSettingsStore((state) => state.accent)

  useEffect(() => {
    let cancelled = false
    const unlisteners: UnlistenFn[] = []

    const keep = (unlisten: UnlistenFn) => {
      if (cancelled) unlisten()
      else unlisteners.push(unlisten)
    }

    void getAppearance().then((next) => {
      if (cancelled) return
      setAppearance(next)
    })

    void onAppearanceChanged(setAppearance).then(keep)

    void onDisplayPreferencesChanged(applyRemote).then(keep)

    void onWindowFocusChanged((focused) => {
      setWindowActive(focused)
      applyWindowActive(focused, document.documentElement)
    }).then(keep)

    return () => {
      cancelled = true
      unlisteners.forEach((unlisten) => {
        unlisten()
      })
    }
  }, [setAppearance, setWindowActive, applyRemote])

  useEffect(() => {
    const preferences = { theme, density, transparency, accent }

    applyAppearance(appearance, preferences, document.documentElement)

    // And the taskbar, which is outside the document and cannot be reached by a token. It is
    // sent the same resolved pair the window is wearing, so the two cannot disagree — the
    // badge used to be a fixed red while the app wore the accent, which is what made it the
    // one thing on screen in the wrong colour.
    const resolved = resolveAccent(appearance, preferences, resolveTheme(appearance, preferences))
    if (resolved !== null) {
      const fill = rgbNumber(resolved)
      const ink = rgbNumber(accentForeground(resolved))
      if (fill !== null && ink !== null) void setBadgePaint(fill, ink)
    }
  }, [appearance, theme, density, transparency, accent])
}
