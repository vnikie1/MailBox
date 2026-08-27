import { StrictMode, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'

import { DEFAULT_APPEARANCE, resolveTheme, type Appearance } from './lib/appearance'
import { useSettingsStore } from './store/settings'
import { App } from './app/App'

import './styles/global.css'

/**
 * Set the appearance before the first paint.
 *
 * Everything else arrives from the core a moment later, but getting the theme wrong for
 * even one frame is a white flash in dark mode, which docs/02 §8 rules out. WebView2
 * reflects the Windows theme in prefers-color-scheme, so the OS half is a real reading
 * rather than a guess, and the IPC read that follows confirms it. The user's half comes
 * from the settings store, which rehydrates from localStorage synchronously for this
 * exact reason. Resolution goes through the same function the running app uses, so the
 * pre-paint frame can never disagree with the frame after it.
 */
const root = document.documentElement
const preferences = useSettingsStore.getState()
const osAppearance: Appearance = {
  ...DEFAULT_APPEARANCE,
  theme: window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
}

root.dataset.theme = resolveTheme(osAppearance, preferences)
root.dataset.density = preferences.density

const container = document.getElementById('root')
if (!container) throw new Error('#root is missing from index.html')

/**
 * The component gallery is a development surface and is not part of the shipped app.
 * Importing it dynamically, behind `import.meta.env.DEV`, keeps it and its stylesheets
 * out of the production bundle entirely rather than relying on tree-shaking to notice —
 * standing rule 18 says no dev scaffolding in shipping code, and this is how that is
 * enforced rather than merely intended.
 */
async function rootElement(): Promise<ReactNode> {
  if (import.meta.env.DEV && window.location.pathname.startsWith('/dev/gallery')) {
    const { Gallery } = await import('./dev/Gallery')
    return <Gallery />
  }

  /**
   * Compose is a separate OS window running the same bundle, told which it is by a query
   * parameter rather than a path. A path would need the dev server and the packaged app to
   * agree on routing for a file that is only ever `index.html`; a query parameter needs
   * neither to know anything.
   *
   * Imported dynamically so the main window does not pay for the editor — Lexical and its
   * plugins are a large chunk that the mailbox never touches.
   */
  if (new URLSearchParams(window.location.search).has('compose')) {
    const { ComposeWindow } = await import('./features/compose/ComposeWindow')
    return <ComposeWindow />
  }

  return <App />
}

createRoot(container).render(<StrictMode>{await rootElement()}</StrictMode>)
