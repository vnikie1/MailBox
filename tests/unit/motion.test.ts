import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'

import { describe, expect, it } from 'vitest'

/**
 * The motion audit, as a test. docs/06 Phase 10.
 *
 * docs/06 asks to "audit every animation against the motion tokens — durations, easing, and no
 * linear stops". An audit performed once is a fact about one afternoon; this is the same audit
 * run on every commit.
 *
 * Stylelint already rejects a raw duration or a raw cubic-bezier in a module. What it cannot
 * see is the rule underneath: **nothing in this app stops linearly.** A linear stop is what
 * makes motion read as mechanical, and it is the one easing that never appears in a system that
 * feels considered.
 *
 * The exception is motion that never stops. A spinner rotating at a constant rate has to be
 * linear — ease it and the rotation visibly stutters once per revolution, because the eye reads
 * the slow part as a hitch. So the rule is precisely "no linear *stop*", and an `infinite`
 * animation is not a stop. Writing it as "no linear anywhere" would have been easier to test
 * and would have made the one place that needs it worse.
 */

function cssFiles(directory: string): string[] {
  const found: string[] = []

  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry)

    if (statSync(path).isDirectory()) {
      found.push(...cssFiles(path))
      continue
    }

    if (entry.endsWith('.css')) found.push(path)
  }

  return found
}

const FILES = cssFiles('src')

describe('motion', () => {
  it('finds stylesheets to audit', () => {
    // Guards the rest: a broken path would make every test below pass over an empty list.
    expect(FILES.length).toBeGreaterThan(10)
  })

  it('never stops linearly', () => {
    for (const file of FILES) {
      const css = readFileSync(file, 'utf8')

      // `linear` inside a gradient is a different property and perfectly fine, so only
      // timing-function positions are checked — and an `infinite` animation is exempt, because
      // it never stops. See the module note.
      const offenders = [...css.matchAll(/(transition|animation)[^;]*;/g)]
        .map((match) => match[0])
        .filter((declaration) => /\blinear\b/.test(declaration))
        .filter((declaration) => !/\binfinite\b/.test(declaration))

      expect(offenders, `${file} stops linearly`).toEqual([])
    }
  })

  it('uses an easing token wherever it eases', () => {
    for (const file of FILES) {
      const css = readFileSync(file, 'utf8')

      const offenders = [...css.matchAll(/(transition|animation)[^;]*;/g)]
        .map((match) => match[0])
        .filter((declaration) => /cubic-bezier|ease-in-out|\bease\b/.test(declaration))
        .filter((declaration) => !declaration.includes('var(--ease'))

      expect(offenders, `${file} eases with a raw curve`).toEqual([])
    }
  })

  it('honours prefers-reduced-motion from one place', () => {
    // Collapsing the duration tokens is what makes this work everywhere at once. A component
    // that animated on a hard-coded duration would ignore the setting silently.
    const primitives = readFileSync('src/styles/tokens/primitive.css', 'utf8')
    expect(primitives).toContain('prefers-reduced-motion')
  })
})
