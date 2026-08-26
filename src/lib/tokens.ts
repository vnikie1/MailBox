/**
 * Reading design tokens back out of the cascade.
 *
 * Two things in this app need a token's *value* in JavaScript rather than in CSS:
 *
 *  - Floating UI takes its offsets as numbers, so a menu that sits 4px from its trigger
 *    would otherwise carry a literal `4` in a component — standing rule 1 with the units
 *    filed off, and invisible to the stylelint rule that enforces it.
 *  - Anything that unmounts after an exit transition needs the duration. The duration
 *    tokens collapse to 0ms under `prefers-reduced-motion` (primitive.css), and a 0ms
 *    transition never fires `transitionend` — so listening for that event would leak the
 *    element forever for exactly the users who asked for less motion. Reading the number
 *    and using a timer is correct in both cases.
 *
 * Both read the computed value, so they stay right when a density mode, a theme or a
 * media query has changed the token underneath.
 */

function computed(token: string, element?: Element): string {
  const target = element ?? document.documentElement
  return getComputedStyle(target).getPropertyValue(token).trim()
}

/** Milliseconds a duration token resolves to, or `fallback` if it is unset or unparseable. */
export function durationToken(token: string, fallback = 0, element?: Element): number {
  const match = /^(-?\d*\.?\d+)(ms|s)$/.exec(computed(token, element))
  if (!match?.[1]) return fallback

  const value = Number.parseFloat(match[1])
  if (!Number.isFinite(value)) return fallback
  return match[2] === 's' ? value * 1000 : value
}

/**
 * Pixels a length token resolves to, or `fallback`.
 *
 * Only `px` is accepted. A token in `em` or `%` has no fixed pixel value outside the
 * element it is used on, and quietly treating "0.5em" as 0.5 would be worse than the
 * fallback.
 */
export function lengthToken(token: string, fallback = 0, element?: Element): number {
  const match = /^(-?\d*\.?\d+)px$/.exec(computed(token, element))
  if (!match?.[1]) return fallback

  const value = Number.parseFloat(match[1])
  return Number.isFinite(value) ? value : fallback
}

export function prefersReducedMotion(): boolean {
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches
}
