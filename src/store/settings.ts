import { create } from 'zustand'
import { persist, createJSONStorage } from 'zustand/middleware'

import {
  DEFAULT_PREFERENCES,
  type Density,
  type AccentPreference,
  type DisplayPreferences,
  type ThemePreference,
  type TransparencyPreference,
} from '@/lib/appearance'
import { broadcastDisplayPreferences, displayPreferencesSet } from '@/lib/ipc'

/**
 * User settings.
 *
 * ## Two stores, and which one is believed
 *
 * The `setting` table in the database is the **source of truth**. `localStorage` is a cache,
 * kept only so the first frame can paint before an IPC round trip could answer.
 *
 * It was the other way round, and it lost the user's settings. WebView2 keeps localStorage in
 * a LevelDB write-ahead log, and on one machine that log became corrupt: LevelDB reported
 * `dropping 3706 bytes; Corruption: checksum mismatch` at the same offset on every launch and
 * discarded everything written past it. The directory had never been compacted, so each run
 * appended into the region the next run would drop — every preference written, none kept.
 *
 * The symptom was precise enough to be worth remembering: the theme came back and the accent
 * did not, because the newest record that survived recovery predated the accent field, and a
 * shallow merge left the accent at its default. A default accent means "use the OS one", which
 * is why it looked like the app was ignoring the choice rather than losing it.
 *
 * Reading the cache synchronously at module load still matters: main.tsx applies the theme
 * and density before the first paint, and an async read would show one frame of the wrong
 * appearance — which is the flash docs/02 §8 rules out. The database value arrives a moment
 * later and corrects the cache if they disagree.
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
  setAccent: (accent: AccentPreference) => void
  /** A change made in another window. Applied, never re-announced. */
  applyRemote: (preferences: DisplayPreferences) => void
}

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set, get) => {
      /** Applies a change here and tells every other window about it. */
      const change = (patch: Partial<DisplayPreferences>) => {
        set(patch)
        const next = displayPreferences(get())

        // Written to the database as well as to localStorage, and the database is the one that
        // is trusted on the next run. `persist` below keeps localStorage in step, but only as
        // a cache for the first frame — see the note on the store.
        void displayPreferencesSet(next)
        void broadcastDisplayPreferences(next)
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
        setAccent: (accent) => {
          change({ accent })
        },
        applyRemote: (preferences) => {
          set(preferences)
        },
      }
    },
    {
      name: 'halcyon.settings.display',
      storage: createJSONStorage(() => localStorage),
      partialize: ({ theme, density, transparency, accent }) => ({
        theme,
        density,
        transparency,
        accent,
      }),
    },
  ),
)

/** The preference triple alone, for the code that resolves it against the OS state. */
export function displayPreferences(state: DisplayPreferences): DisplayPreferences {
  return {
    theme: state.theme,
    density: state.density,
    transparency: state.transparency,
    // No defaulting needed for an install that predates this field: zustand shallow-merges
    // what it read over the initial state, so a persisted object with no accent key keeps the
    // one DEFAULT_PREFERENCES put there.
    accent: state.accent,
  }
}
