import { useEffect } from 'react'
import type { UnlistenFn } from '@tauri-apps/api/event'

import { applyAppearance, applyWindowActive } from '@/lib/appearance'
import { getAppearance, onAppearanceChanged, onWindowFocusChanged } from '@/lib/ipc'
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
 */
export function useAppearanceSync(): void {
  const setAppearance = useAppearanceStore((state) => state.setAppearance)
  const setWindowActive = useAppearanceStore((state) => state.setWindowActive)
  const appearance = useAppearanceStore((state) => state.appearance)

  // Selected field by field rather than as an object: a fresh object every render would
  // re-run the effect below on every render.
  const theme = useSettingsStore((state) => state.theme)
  const density = useSettingsStore((state) => state.density)
  const transparency = useSettingsStore((state) => state.transparency)

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
  }, [setAppearance, setWindowActive])

  useEffect(() => {
    applyAppearance(appearance, { theme, density, transparency }, document.documentElement)
  }, [appearance, theme, density, transparency])
}
