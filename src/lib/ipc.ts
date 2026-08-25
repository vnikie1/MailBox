/**
 * The IPC client. docs/03-architecture.md §4.
 *
 * The UI reaches the core through this module and nowhere else — standing rule 9 means
 * there is never a network call on this side of the seam, and standing rule 14 means
 * changes arrive as events rather than by polling.
 *
 * Every entry point also has a browser path. That is not a mock of the app: it is what
 * the shell genuinely is when served by Vite rather than hosted in a WebView, and it is
 * what Playwright and the Phase 1 component gallery run against. Where the browser can
 * answer honestly (the OS theme, reduced transparency, window focus) it does; where it
 * cannot (a DWM material) it reports the truth, which is that there is none.
 */

import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { DEFAULT_APPEARANCE, type Appearance, type ThemeName } from './appearance'

export const runningInTauri: boolean = isTauri()

/* ------------------------------------------------------------------ appearance */

function browserAppearance(): Appearance {
  const dark = window.matchMedia('(prefers-color-scheme: dark)').matches
  const reduceTransparency = window.matchMedia('(prefers-reduced-transparency: reduce)').matches
  return {
    ...DEFAULT_APPEARANCE,
    theme: (dark ? 'dark' : 'light') satisfies ThemeName,
    reduceTransparency,
  }
}

export async function getAppearance(): Promise<Appearance> {
  if (!runningInTauri) return browserAppearance()
  return invoke<Appearance>('appearance_get')
}

export async function onAppearanceChanged(
  handler: (appearance: Appearance) => void,
): Promise<UnlistenFn> {
  if (!runningInTauri) {
    const queries = [
      window.matchMedia('(prefers-color-scheme: dark)'),
      window.matchMedia('(prefers-reduced-transparency: reduce)'),
    ]
    const relay = () => {
      handler(browserAppearance())
    }
    queries.forEach((q) => {
      q.addEventListener('change', relay)
    })
    return () => {
      queries.forEach((q) => {
        q.removeEventListener('change', relay)
      })
    }
  }
  return listen<Appearance>('system:appearance', (event) => {
    handler(event.payload)
  })
}

/* --------------------------------------------------------------- window chrome */

export async function onWindowFocusChanged(
  handler: (focused: boolean) => void,
): Promise<UnlistenFn> {
  if (!runningInTauri) {
    const onFocus = () => {
      handler(true)
    }
    const onBlur = () => {
      handler(false)
    }
    window.addEventListener('focus', onFocus)
    window.addEventListener('blur', onBlur)
    return () => {
      window.removeEventListener('focus', onFocus)
      window.removeEventListener('blur', onBlur)
    }
  }
  return getCurrentWindow().onFocusChanged(({ payload }) => {
    handler(payload)
  })
}
