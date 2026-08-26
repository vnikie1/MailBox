/**
 * A seeded pseudo-random generator.
 *
 * `Math.random()` would make every run of the fixture generator produce a different inbox,
 * which breaks the Playwright visual baselines the moment anything re-renders and makes
 * "the list looks wrong" impossible to reproduce. mulberry32 is four lines, has a long
 * enough period for two thousand messages, and is stable across platforms — which
 * `Math.random()` is not, even with a seed.
 */
export function createRandom(seed: number): () => number {
  let state = seed >>> 0

  return function next(): number {
    state = (state + 0x6d2b79f5) >>> 0
    let t = state
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

export interface Rng {
  /** A float in [0, 1). */
  next: () => number
  /** An integer in [min, max], inclusive at both ends. */
  int: (min: number, max: number) => number
  /** One element. Throws on an empty list rather than returning undefined. */
  pick: <T>(items: readonly T[]) => T
  /** True with the given probability. */
  chance: (probability: number) => boolean
  /** A new array in a shuffled order; the input is not mutated. */
  shuffle: <T>(items: readonly T[]) => T[]
}

export function createRng(seed: number): Rng {
  const next = createRandom(seed)

  const int = (min: number, max: number) => min + Math.floor(next() * (max - min + 1))

  const pick = <T>(items: readonly T[]): T => {
    const item = items[int(0, items.length - 1)]
    if (item === undefined) throw new Error('cannot pick from an empty list')
    return item
  }

  const shuffle = <T>(items: readonly T[]): T[] => {
    const copy = [...items]
    for (let i = copy.length - 1; i > 0; i -= 1) {
      const j = int(0, i)
      const a = copy[i]
      const b = copy[j]
      if (a !== undefined && b !== undefined) {
        copy[i] = b
        copy[j] = a
      }
    }
    return copy
  }

  return { next, int, pick, chance: (probability) => next() < probability, shuffle }
}
