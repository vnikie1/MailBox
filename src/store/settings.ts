import { create } from 'zustand'
import { persist, createJSONStorage } from 'zustand/middleware'

import {
  DEFAULT_PREFERENCES,
  type Density,
  type DisplayPreferences,
  type ThemePreference,
  type TransparencyPreference,
} from '@/lib/appearance'

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
 */
interface SettingsState extends DisplayPreferences {
  setTheme: (theme: ThemePreference) => void
  setDensity: (density: Density) => void
  setTransparency: (transparency: TransparencyPreference) => void
}

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      ...DEFAULT_PREFERENCES,
      setTheme: (theme) => {
        set({ theme })
      },
      setDensity: (density) => {
        set({ density })
      },
      setTransparency: (transparency) => {
        set({ transparency })
      },
    }),
    {
      name: 'halcyon.settings.display',
      storage: createJSONStorage(() => localStorage),
      partialize: ({ theme, density, transparency }) => ({ theme, density, transparency }),
    },
  ),
)

/** The preference triple alone, for the code that resolves it against the OS state. */
export function displayPreferences(state: SettingsState): DisplayPreferences {
  return { theme: state.theme, density: state.density, transparency: state.transparency }
}
