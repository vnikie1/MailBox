import { beforeEach, describe, expect, it } from 'vitest'

import {
  accentForeground,
  applyAppearance,
  applyWindowActive,
  parseHexColour,
  relativeLuminance,
  DEFAULT_PREFERENCES,
  resolveReduceTransparency,
  resolveTheme,
  type Appearance,
} from '@/lib/appearance'

describe('parseHexColour', () => {
  it('accepts six-digit hex with or without the hash, in either case', () => {
    expect(parseHexColour('#007AFF')).toEqual([0x00, 0x7a, 0xff])
    expect(parseHexColour('007aff')).toEqual([0x00, 0x7a, 0xff])
    expect(parseHexColour('  #FFCC00  ')).toEqual([0xff, 0xcc, 0x00])
  })

  it('rejects anything else rather than guessing', () => {
    expect(parseHexColour('#fff')).toBeNull()
    expect(parseHexColour('#00112233')).toBeNull()
    expect(parseHexColour('rgb(0,0,0)')).toBeNull()
    expect(parseHexColour('')).toBeNull()
  })
})

describe('relativeLuminance', () => {
  it('anchors at black and white', () => {
    expect(relativeLuminance('#000000')).toBeCloseTo(0, 5)
    expect(relativeLuminance('#FFFFFF')).toBeCloseTo(1, 5)
  })

  it('matches the WCAG value for the Apple accent palette', () => {
    // Spot-checked against the WCAG 2.x relative-luminance definition.
    expect(relativeLuminance('#007AFF')).toBeCloseTo(0.2113, 3)
    expect(relativeLuminance('#FFCC00')).toBeCloseTo(0.6444, 3)
  })

  it('returns null for an unparseable colour instead of NaN', () => {
    expect(relativeLuminance('nonsense')).toBeNull()
  })
})

describe('accentForeground', () => {
  /**
   * The point of these cases: a WCAG-maximising rule would put BLACK on Apple blue
   * (5.2:1 vs 4.0:1), which would destroy the look the project exists to reproduce.
   * The threshold keeps white there and flips only genuinely light accents.
   */
  it('keeps white text on the accents macOS uses white on', () => {
    for (const accent of ['#007AFF', '#0A84FF', '#FF3B30', '#AF52DE', '#5856D6', '#FF2D55']) {
      expect(accentForeground(accent), accent).toBe('#FFFFFF')
    }
  })

  it('flips to black on accents where white would be unreadable', () => {
    expect(accentForeground('#FFCC00')).toBe('#000000')
    expect(accentForeground('#FFD60A')).toBe('#000000')
    expect(accentForeground('#FFFFFF')).toBe('#000000')
  })

  it('falls back to white when the accent cannot be parsed', () => {
    expect(accentForeground('not a colour')).toBe('#FFFFFF')
  })
})

describe('applyAppearance', () => {
  let root: HTMLElement

  const base: Appearance = {
    theme: 'light',
    accent: null,
    reduceTransparency: false,
    backdrop: 'none',
  }

  beforeEach(() => {
    root = document.createElement('html')
  })

  it('writes theme and backdrop as data attributes', () => {
    applyAppearance({ ...base, theme: 'dark', backdrop: 'micaAlt' }, DEFAULT_PREFERENCES, root)
    expect(root.dataset.theme).toBe('dark')
    expect(root.dataset.backdrop).toBe('micaAlt')
  })

  it('adds and removes the reduce-transparency flag', () => {
    applyAppearance({ ...base, reduceTransparency: true }, DEFAULT_PREFERENCES, root)
    expect(root.dataset.reduceTransparency).toBe('')

    applyAppearance({ ...base, reduceTransparency: false }, DEFAULT_PREFERENCES, root)
    expect(root.dataset.reduceTransparency).toBeUndefined()
  })

  it('sets the accent pair together', () => {
    applyAppearance({ ...base, accent: '#FFCC00' }, DEFAULT_PREFERENCES, root)
    expect(root.style.getPropertyValue('--accent-system')).toBe('#FFCC00')
    expect(root.style.getPropertyValue('--accent-fg-system')).toBe('#000000')
  })

  it('leaves the accent properties unset when Windows reports no accent', () => {
    // Unset is meaningful: semantic.css then falls back to the Apple blue pair, so the
    // fallback value is never duplicated in two places.
    applyAppearance({ ...base, accent: '#007AFF' }, DEFAULT_PREFERENCES, root)
    applyAppearance({ ...base, accent: null }, DEFAULT_PREFERENCES, root)
    expect(root.style.getPropertyValue('--accent-system')).toBe('')
    expect(root.style.getPropertyValue('--accent-fg-system')).toBe('')
  })
})

describe('applyWindowActive', () => {
  it('marks the document only while the window is inactive', () => {
    const root = document.createElement('html')

    applyWindowActive(false, root)
    expect(root.dataset.windowInactive).toBe('')

    applyWindowActive(true, root)
    expect(root.dataset.windowInactive).toBeUndefined()
  })
})

describe('preference resolution', () => {
  const os: Appearance = {
    theme: 'dark',
    accent: null,
    reduceTransparency: true,
    backdrop: 'micaAlt',
  }

  it('follows the OS by default and obeys a pinned theme', () => {
    expect(resolveTheme(os, DEFAULT_PREFERENCES)).toBe('dark')
    expect(resolveTheme(os, { ...DEFAULT_PREFERENCES, theme: 'light' })).toBe('light')
    expect(resolveTheme(os, { ...DEFAULT_PREFERENCES, theme: 'dark' })).toBe('dark')
  })

  it('lets the user turn transparency back on when Windows has turned it off', () => {
    // The Windows setting is system-wide; a user who wants the app translucent should not
    // have to change it for every app. docs/02 §5 asks for the in-app toggle for this.
    expect(resolveReduceTransparency(os, DEFAULT_PREFERENCES)).toBe(true)
    expect(resolveReduceTransparency(os, { ...DEFAULT_PREFERENCES, transparency: 'full' })).toBe(
      false,
    )

    const opaqueOs: Appearance = { ...os, reduceTransparency: false }
    expect(
      resolveReduceTransparency(opaqueOs, { ...DEFAULT_PREFERENCES, transparency: 'reduce' }),
    ).toBe(true)
  })

  it('writes the resolved theme and the density together', () => {
    const root = document.createElement('html')

    applyAppearance(os, { theme: 'light', density: 'compact', transparency: 'full' }, root)

    expect(root.dataset.theme).toBe('light')
    expect(root.dataset.density).toBe('compact')
    expect(root.dataset.reduceTransparency).toBeUndefined()
  })
})
