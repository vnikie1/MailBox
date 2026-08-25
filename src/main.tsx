import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { App } from './app/App'

import './styles/global.css'

/**
 * Set the theme before the first paint.
 *
 * Everything else about appearance arrives from the core a moment later, but getting the
 * theme wrong for even one frame is a white flash in dark mode, which docs/02 §8 rules
 * out. WebView2 reflects the Windows theme in prefers-color-scheme, so this is correct
 * rather than merely a guess, and the IPC read that follows confirms it.
 */
const root = document.documentElement
root.dataset.theme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
root.dataset.density = 'default'

const container = document.getElementById('root')
if (!container) throw new Error('#root is missing from index.html')

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
