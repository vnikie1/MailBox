import { create } from 'zustand'

import { DEFAULT_APPEARANCE, type Appearance } from '@/lib/appearance'

interface AppearanceState {
  appearance: Appearance
  /** docs/01 §9.11 — the window goes quiet when it is not the active one. */
  windowActive: boolean
  setAppearance: (appearance: Appearance) => void
  setWindowActive: (active: boolean) => void
}

export const useAppearanceStore = create<AppearanceState>()((set) => ({
  appearance: DEFAULT_APPEARANCE,
  windowActive: true,
  setAppearance: (appearance) => {
    set({ appearance })
  },
  setWindowActive: (windowActive) => {
    set({ windowActive })
  },
}))
