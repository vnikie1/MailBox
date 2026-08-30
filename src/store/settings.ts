import { create } from 'zustand'
import { persist, createJSONStorage } from 'zustand/middleware'

import {
  DEFAULT_PREFERENCES,
  type Density,
  type DisplayPreferences,
  type ThemePreference,
  type TransparencyPreference,
} from '@/lib/appearance'
import { broadcastDisplayPreferences } from '@/lib/ipc'

/**
 * User settings.
 *
 * Persisted to the WebView's localStorage, which in the packaged app lives under the
 * bundle identifier's data directory and survives restart. Phase 3 moves this into the
 * `settings` table so it is backed up and synced with everything else; the shape here is
 * chosen to make that a straight port rather than a rewrite.
 *
 * Reading it synchronously at module load matters: main.tsx applies the persisted theme
 * and density before the first paint, and an async rehydration would show one frame of
 * the wrong appearance — which is the flash docs/02 §8 rules out.
 *
 * ## Two ways in, and why
 *
 * Since Phase 11 these three live in a window of their own. A setter announces the change to
 * every other window; `applyRemote` is how a window takes one in. They are separate functions
 * rather than one with a flag because a single one would announce what it had just been told,
 * and two windows would talk to each other for ever.
 */
interface SettingsState extends DisplayPreferences {
  setTheme: (theme: ThemePreference) => void
  setDensity: (density: Density) => void
  setTransparency: (transparency: TransparencyPreference) => void
  /** A change made in another window. Applied, never re-announced. */
  applyRemote: (preferences: DisplayPreferences) => void
}

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set, get) => {
      /** Applies a change here and tells every other window about it. */
      const change = (patch: Partial<DisplayPreferences>) => {
        set(patch)
        void broadcastDisplayPreferences(displayPreferences(get()))
      }

      return {
        ...DEFAULT_PREFERENCES,
        setTheme: (theme) => {
          change({ theme })
        },
        setDensity: (density) => {
          change({ density })
        },
        setTransparency: (transparency) => {
          change({ transparency })
        },
        applyRemote: (preferences) => {
          set(preferences)
        },
      }
    },
    {
      name: 'halcyon.settings.display',
      storage: createJSONStorage(() => localStorage),
      partialize: ({ theme, density, transparency }) => ({ theme, density, transparency }),
    },
  ),
)

/** The preference triple alone, for the code that resolves it against the OS state. */
export function displayPreferences(state: DisplayPreferences): DisplayPreferences {
  return { theme: state.theme, density: state.density, transparency: state.transparency }
}
