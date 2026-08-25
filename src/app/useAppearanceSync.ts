import { useEffect } from 'react'
import type { UnlistenFn } from '@tauri-apps/api/event'

import { applyAppearance, applyWindowActive } from '@/lib/appearance'
import { getAppearance, onAppearanceChanged, onWindowFocusChanged } from '@/lib/ipc'
import { useAppearanceStore } from '@/store/appearance'

/**
 * Keep the document in step with the OS appearance.
 *
 * The core pushes changes; nothing here polls (standing rule 14). The initial read is the
 * one and only request — every subsequent update arrives on the event.
 */
export function useAppearanceSync(): void {
  const setAppearance = useAppearanceStore((state) => state.setAppearance)
  const setWindowActive = useAppearanceStore((state) => state.setWindowActive)

  useEffect(() => {
    let cancelled = false
    const unlisteners: UnlistenFn[] = []

    const keep = (unlisten: UnlistenFn) => {
      if (cancelled) unlisten()
      else unlisteners.push(unlisten)
    }

    void getAppearance().then((appearance) => {
      if (cancelled) return
      setAppearance(appearance)
      applyAppearance(appearance, document.documentElement)
    })

    void onAppearanceChanged((appearance) => {
      setAppearance(appearance)
      applyAppearance(appearance, document.documentElement)
    }).then(keep)

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
}
