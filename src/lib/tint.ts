/**
 * Which of the eight contact tints an address gets. docs/01 §4 — "a colour derived
 * deterministically from the address hash".
 *
 * FNV-1a: small, well-distributed for short strings, and stable across platforms, which a
 * hash built from summing `charCodeAt` is not — that one collides for any two contacts
 * whose addresses are anagrams, which is commoner than it sounds among firstname.lastname
 * addresses at one domain.
 *
 * Lives outside the Avatar component because a module that exports both a component and a
 * function loses fast refresh for everything in it, which the react-refresh rule enforces.
 */
export const TINT_COUNT = 8

export function tintIndexFor(key: string): number {
  let hash = 0x811c9dc5

  for (let i = 0; i < key.length; i += 1) {
    hash ^= key.charCodeAt(i)
    hash = Math.imul(hash, 0x01000193)
  }

  return Math.abs(hash) % TINT_COUNT
}
