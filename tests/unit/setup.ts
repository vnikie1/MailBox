import '@testing-library/jest-dom/vitest'

/**
 * jsdom implements neither of these, and both are load-bearing for the shell: matchMedia
 * is how the browser path reads the OS theme, and ResizeObserver is how the caption
 * buttons notice that they have been relaid out.
 */

if (typeof window.matchMedia !== 'function') {
  window.matchMedia = (query: string): MediaQueryList => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => false,
  })
}

if (typeof globalThis.ResizeObserver !== 'function') {
  globalThis.ResizeObserver = class {
    observe() {
      return undefined
    }
    unobserve() {
      return undefined
    }
    disconnect() {
      return undefined
    }
  }
}

/**
 * Floating UI's `autoUpdate` observes intersection as well as resize, and
 * `useListNavigation` scrolls the active item into view. jsdom implements neither, and
 * without them every test that opens a menu or a popover throws before it asserts
 * anything.
 */

if (typeof globalThis.IntersectionObserver !== 'function') {
  globalThis.IntersectionObserver = class {
    readonly root = null
    readonly rootMargin = ''
    readonly thresholds: readonly number[] = []
    observe() {
      return undefined
    }
    unobserve() {
      return undefined
    }
    disconnect() {
      return undefined
    }
    takeRecords(): IntersectionObserverEntry[] {
      return []
    }
  }
}

if (typeof Element.prototype.scrollIntoView !== 'function') {
  Element.prototype.scrollIntoView = function scrollIntoView() {
    return undefined
  }
}
