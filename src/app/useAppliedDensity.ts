import { useSyncExternalStore } from 'react'

/**
 * The density the document is drawn at: `<html data-density>`, as `applyAppearance` last wrote it.
 *
 * Not the density in the settings store. The store changes first, and the attribute — which is
 * what the CSS custom properties key off — is written afterwards, in an effect of the root
 * component, so anything that renders on the store's change and reads a token sees the old one.
 * The message list did exactly that: it measured its row height on the render the density
 * changed, got the previous density's, and laid every row out one step behind until something
 * else made it measure again, which in practice was a reload (L19, 2026-11-14 audit).
 *
 * Watching the attribute itself means the render this causes comes after the tokens have moved.
 */
export function useAppliedDensity(): string {
  return useSyncExternalStore(subscribe, read, read)
}

function read(): string {
  return document.documentElement.dataset.density ?? ''
}

function subscribe(onChange: () => void): () => void {
  const observer = new MutationObserver(onChange)
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-density'],
  })
  return () => {
    observer.disconnect()
  }
}
