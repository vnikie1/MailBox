import { describe, expect, it } from 'vitest'

import { looksLikeAddress } from '@/features/compose/address'

/**
 * What the compose field colours red.
 *
 * The check is deliberately loose — the core validates properly, and this only decides whether
 * a chip looks wrong to the user. Being loose is fine. Being wrong about the ordinary case is
 * not, and it was: the check tested the whole token, and the token is not always a bare
 * address.
 */
describe('a recipient chip', () => {
  it('accepts an address picked from the autocomplete', () => {
    // `RecipientField` commits suggestions in the full form on purpose, so the message carries
    // the name the mailbox already knows. That form contains a space, the whitespace test
    // failed on it, and every suggested recipient came out red.
    expect(looksLikeAddress('Ada Lovelace <ada@example.test>')).toBe(true)
    expect(looksLikeAddress('"Lovelace, Ada" <ada@example.test>')).toBe(true)
    expect(looksLikeAddress('Zoë Naïve <zoe@example.test>')).toBe(true)
  })

  it('accepts a bare address', () => {
    expect(looksLikeAddress('ada@example.test')).toBe(true)
  })

  it('still rejects what is plainly not an address', () => {
    expect(looksLikeAddress('')).toBe(false)
    expect(looksLikeAddress('ada')).toBe(false)
    expect(looksLikeAddress('@example.test')).toBe(false)
    expect(looksLikeAddress('ada@')).toBe(false)
    // Two addresses in one chip: whatever the user meant, it is not one recipient.
    expect(looksLikeAddress('ada@example.test bob@example.test')).toBe(false)
  })

  it('rejects a bracketed value whose address is not one', () => {
    // The brackets must not be a way to smuggle anything through the check.
    expect(looksLikeAddress('Ada Lovelace <not an address>')).toBe(false)
    expect(looksLikeAddress('Ada Lovelace <>')).toBe(false)
  })
})
