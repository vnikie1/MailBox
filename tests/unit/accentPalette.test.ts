import { readFileSync } from 'node:fs'

import { describe, expect, it } from 'vitest'

import { ACCENT_NAMES, ACCENT_PALETTE, type AccentName } from '@/lib/appearance'

/**
 * The accent palette exists twice, and this is what stops the copies drifting.
 *
 * `primitive.css` holds the eleven Apple system pairs as the design system's own record of
 * them; `ACCENT_PALETTE` holds the same values in TypeScript because the accent has to be
 * resolved to a value before first paint and handed to the taskbar badge, and neither of those
 * can read a CSS custom property.
 *
 * Two enumerations that cannot import each other are exactly the shape of thing that rots: a
 * hue added on one side renders a colourless swatch, and a value edited on one side gives a
 * picker whose preview does not match what it sets. Neither would fail a type check, and
 * neither would look wrong in review. `tests/unit/settings.test.ts` already uses this idiom to
 * keep the settings panes aligned with the Rust source.
 */

// Repo-relative, as tests/unit/settings.test.ts reads the Rust source: vitest runs from the
// project root.
const CSS = readFileSync('src/styles/tokens/primitive.css', 'utf8')

/** Every `--<name>-l` / `--<name>-d` pair declared in the primitive layer. */
function palettePairsFromCss(): Map<string, { light: string; dark: string }> {
  const pairs = new Map<string, { light: string; dark: string }>()

  for (const match of CSS.matchAll(/--([a-z]+)-([ld]):\s*(#[0-9a-f]{6})\s*;/g)) {
    const [, name, suffix, value] = match
    if (name === undefined || suffix === undefined || value === undefined) continue

    const entry = pairs.get(name) ?? { light: '', dark: '' }
    if (suffix === 'l') entry.light = value
    else entry.dark = value
    pairs.set(name, entry)
  }

  return pairs
}

describe('the accent palette', () => {
  const fromCss = palettePairsFromCss()

  it('reads the pairs it is being checked against', () => {
    // Guards the test itself. A regex that matched nothing would make every assertion below
    // vacuously true, which is the failure mode of every parse-the-source test.
    expect(fromCss.size).toBeGreaterThanOrEqual(11)
    expect(fromCss.get('blue')).toEqual({ light: '#007aff', dark: '#0a84ff' })
  })

  it('offers the eleven hues docs/01 §11 names', () => {
    expect(ACCENT_NAMES).toEqual([
      'blue',
      'red',
      'orange',
      'yellow',
      'green',
      'mint',
      'teal',
      'indigo',
      'purple',
      'pink',
      'gray',
    ])
  })

  it.each(Object.keys(ACCENT_PALETTE) as AccentName[])(
    '%s matches primitive.css in both themes',
    (name) => {
      const css = fromCss.get(name)

      expect(css, `--${name}-l / --${name}-d are not in primitive.css`).toBeDefined()
      expect(ACCENT_PALETTE[name].light).toBe(css?.light)
      expect(ACCENT_PALETTE[name].dark).toBe(css?.dark)
    },
  )

  it('gives every hue a distinct value in each theme', () => {
    // Two hues sharing a value would make one of them unselectable in practice: the swatch
    // would light up for the wrong one, because the resolved value is what gets stored.
    const light = new Set(Object.values(ACCENT_PALETTE).map((pair) => pair.light))
    const dark = new Set(Object.values(ACCENT_PALETTE).map((pair) => pair.dark))

    expect(light.size).toBe(ACCENT_NAMES.length)
    expect(dark.size).toBe(ACCENT_NAMES.length)
  })
})
