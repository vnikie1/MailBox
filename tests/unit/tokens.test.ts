import { afterEach, describe, expect, it, vi } from 'vitest'

import { durationToken, lengthToken } from '@/lib/tokens'

/**
 * jsdom does not resolve custom properties through getComputedStyle, so these stub it.
 * That is the right call regardless: what is under test is the parsing, and the fallback
 * behaviour, not whether a browser can read its own cascade.
 */
function withComputedValue(value: string) {
  vi.spyOn(window, 'getComputedStyle').mockReturnValue({
    getPropertyValue: () => value,
  } as unknown as CSSStyleDeclaration)
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('durationToken', () => {
  it('reads milliseconds and seconds', () => {
    withComputedValue('250ms')
    expect(durationToken('--dur-slow')).toBe(250)

    withComputedValue('1.2s')
    expect(durationToken('--dur-shimmer')).toBe(1200)
  })

  it('reads the zero the reduced-motion media query writes', () => {
    // This is the case that matters: primitive.css collapses every duration to 0ms under
    // prefers-reduced-motion, and anything scheduling an unmount has to see the 0 rather
    // than quietly falling back to its default.
    withComputedValue('0ms')
    expect(durationToken('--dur-base', 200)).toBe(0)
  })

  it('falls back when the token is unset or is not a duration', () => {
    withComputedValue('')
    expect(durationToken('--nope', 42)).toBe(42)

    withComputedValue('var(--dur-base)')
    expect(durationToken('--unresolved', 42)).toBe(42)

    withComputedValue('fast')
    expect(durationToken('--nonsense', 42)).toBe(42)
  })
})

describe('lengthToken', () => {
  it('reads pixel lengths', () => {
    withComputedValue('12px')
    expect(lengthToken('--sp-6')).toBe(12)

    withComputedValue('0.5px')
    expect(lengthToken('--hairline-half')).toBe(0.5)
  })

  it('refuses relative units instead of guessing a pixel value for them', () => {
    // "0.5em" has no fixed pixel value outside the element it is used on, and treating
    // the number as pixels would place a menu half a pixel from its trigger.
    withComputedValue('0.5em')
    expect(lengthToken('--relative', 8)).toBe(8)

    withComputedValue('50%')
    expect(lengthToken('--proportional', 8)).toBe(8)
  })
})
