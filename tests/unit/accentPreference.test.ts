import { describe, expect, it } from 'vitest'

import {
  ACCENT_PALETTE,
  DEFAULT_APPEARANCE,
  DEFAULT_PREFERENCES,
  accentForeground,
  applyAppearance,
  resolveAccent,
  rgbNumber,
  type Appearance,
  type DisplayPreferences,
} from '@/lib/appearance'

/**
 * Choosing an accent, and what the rest of the app is told about it.
 *
 * The accent already had a resolution layer — Windows reports one, the user may override it,
 * and `applyAppearance` writes the answer to `<html>`. This adds the override, so what is worth
 * pinning is the *precedence* and the *theme pairing*: a pinned hue has a light and a dark
 * member, and picking the wrong one is the difference between a mint that reads and one that
 * does not.
 */

function appearanceWith(accent: string | null, theme: 'light' | 'dark' = 'light'): Appearance {
  return { ...DEFAULT_APPEARANCE, accent, theme }
}

function preferencesWith(patch: Partial<DisplayPreferences>): DisplayPreferences {
  return { ...DEFAULT_PREFERENCES, ...patch }
}

describe('resolving the accent', () => {
  it('follows Windows by default', () => {
    expect(DEFAULT_PREFERENCES.accent).toBe('system')

    const resolved = resolveAccent(appearanceWith('#F7630C'), preferencesWith({}), 'light')

    expect(resolved).toBe('#F7630C')
  })

  it('prefers a pinned colour over the one Windows reports', () => {
    const resolved = resolveAccent(
      appearanceWith('#F7630C'),
      preferencesWith({ accent: 'purple' }),
      'light',
    )

    expect(resolved).toBe(ACCENT_PALETTE.purple.light)
  })

  it('takes the member of the pair that matches the resolved theme', () => {
    // The whole reason the palette holds pairs. Mint is the case that proves it: the light
    // member wants white text on it and the dark member wants black.
    const light = resolveAccent(appearanceWith(null), preferencesWith({ accent: 'mint' }), 'light')
    const dark = resolveAccent(appearanceWith(null), preferencesWith({ accent: 'mint' }), 'dark')

    expect(light).toBe('#00c7be')
    expect(dark).toBe('#63e6e2')
    expect(light).not.toBe(dark)

    expect(accentForeground('#00c7be')).toBe('#FFFFFF')
    expect(accentForeground('#63e6e2')).toBe('#000000')
  })

  it('still resolves a pinned colour when Windows reports none', () => {
    // A pinned accent is the user's answer, not a refinement of the OS's. It has to survive a
    // machine that will not report one at all.
    expect(resolveAccent(appearanceWith(null), preferencesWith({ accent: 'teal' }), 'dark')).toBe(
      ACCENT_PALETTE.teal.dark,
    )
  })

  it('reports nothing when nothing is pinned and Windows is silent', () => {
    // Null rather than a hardcoded blue: semantic.css owns the fallback, and spelling it out
    // here would put the same value in two places.
    expect(resolveAccent(appearanceWith(null), preferencesWith({}), 'light')).toBeNull()
  })
})

describe('applying the accent to the document', () => {
  it('writes the pinned colour and a foreground that reads on it', () => {
    const root = document.createElement('html')

    applyAppearance(
      appearanceWith('#F7630C', 'dark'),
      preferencesWith({ accent: 'yellow', theme: 'dark' }),
      root,
    )

    // Yellow is the case the contrast rule exists for: white on #ffd60a is about 1.5:1.
    expect(root.style.getPropertyValue('--accent-system')).toBe('#ffd60a')
    expect(root.style.getPropertyValue('--accent-fg-system')).toBe('#000000')
  })

  it('leaves both properties unset when there is nothing to say', () => {
    const root = document.createElement('html')
    root.style.setProperty('--accent-system', '#123456')

    applyAppearance(appearanceWith(null), preferencesWith({}), root)

    expect(root.style.getPropertyValue('--accent-system')).toBe('')
    expect(root.style.getPropertyValue('--accent-fg-system')).toBe('')
  })

  it('changes the accent without disturbing the theme it was resolved against', () => {
    // Guards the ordering inside applyAppearance: the accent is resolved against the theme the
    // same call just computed, so a pinned dark accent cannot land on a light document.
    const root = document.createElement('html')

    applyAppearance(
      appearanceWith('#F7630C', 'light'),
      preferencesWith({ accent: 'indigo', theme: 'dark' }),
      root,
    )

    expect(root.dataset.theme).toBe('dark')
    expect(root.style.getPropertyValue('--accent-system')).toBe(ACCENT_PALETTE.indigo.dark)
  })
})

describe('handing the accent to the taskbar', () => {
  it('packs a hex string the way the badge expects it', () => {
    // 0x00RRGGBB. Getting the channel order wrong would draw the badge in a colour nobody
    // chose, and it is the kind of mistake that looks plausible on screen.
    expect(rgbNumber('#F7630C')).toBe(0xf7630c)
    expect(rgbNumber('#000000')).toBe(0)
    expect(rgbNumber('#ffffff')).toBe(0xffffff)
  })

  it('refuses a value it cannot parse rather than guessing', () => {
    expect(rgbNumber('rebeccapurple')).toBeNull()
    expect(rgbNumber('#12345')).toBeNull()
  })
})
