import { describe, expect, it } from 'vitest'

import { GROUP_ORDER, SHORTCUTS, matches, parseChord, type ShortcutId } from '@/app/shortcuts'

/**
 * The shortcut registry. docs/01 §14, docs/06 Phase 10.
 *
 * The collision test is the one that earns its keep. Two handlers claiming the same chord used
 * to be undetectable — both would run, or one would swallow the other depending on mount order,
 * and nothing anywhere would say so.
 */

/** A key event as the dispatcher sees it. */
function press(key: string, modifiers: { ctrl?: boolean; shift?: boolean; alt?: boolean } = {}) {
  return {
    key,
    ctrlKey: modifiers.ctrl ?? false,
    shiftKey: modifiers.shift ?? false,
    altKey: modifiers.alt ?? false,
  } as KeyboardEvent
}

describe('the registry', () => {
  it('binds no chord twice', () => {
    const seen = new Map<string, string>()

    for (const shortcut of SHORTCUTS) {
      const chord = parseChord(shortcut.keys)
      if (chord === null) continue

      const signature = `${String(chord.ctrl)}-${String(chord.shift)}-${String(chord.alt)}-${chord.key}`
      const existing = seen.get(signature)

      expect(
        existing,
        `${shortcut.keys} is claimed by both ${String(existing)} and ${shortcut.id}`,
      ).toBeUndefined()

      seen.set(signature, shortcut.id)
    }
  })

  it('gives every shortcut a unique id', () => {
    const ids = SHORTCUTS.map((shortcut) => shortcut.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('puts every shortcut in a group the reference sheet renders', () => {
    // A shortcut in a group Help does not know about would exist and be undiscoverable.
    for (const shortcut of SHORTCUTS) {
      expect(GROUP_ORDER, `${shortcut.id} is in an unrendered group`).toContain(shortcut.group)
    }
  })

  it('covers every action docs/01 §14 lists', () => {
    // The spec says "ship all of these". This is that list, so dropping one is a test failure
    // rather than something noticed a phase later.
    const required = [
      'Ctrl+N',
      'Ctrl+Enter',
      'Ctrl+R',
      'Ctrl+Shift+R',
      'Ctrl+Shift+F',
      'Ctrl+Shift+E',
      'Ctrl+Shift+A',
      'Delete',
      'Shift+Delete',
      'Ctrl+U',
      'Ctrl+L',
      'Ctrl+J',
      'Ctrl+Shift+M',
      'Ctrl+F',
      'F5',
      'Ctrl+Shift+S',
      'Ctrl+Z',
    ]

    const present = new Set(SHORTCUTS.map((shortcut) => shortcut.keys))
    for (const keys of required) {
      expect(present, `docs/01 §14 requires ${keys}`).toContain(keys)
    }
  })
})

describe('parsing a chord', () => {
  it('reads the modifiers and the key', () => {
    expect(parseChord('Ctrl+Shift+M')).toEqual({ ctrl: true, shift: true, alt: false, key: 'm' })
    expect(parseChord('Delete')).toEqual({ ctrl: false, shift: false, alt: false, key: 'delete' })
    expect(parseChord('Alt+Ctrl+L')).toEqual({ ctrl: true, shift: false, alt: true, key: 'l' })
  })

  it('refuses what is not one literal key', () => {
    // Reference-sheet entries, not bindings: a range, and arrows the list owns.
    expect(parseChord('Ctrl+1–9')).toBeNull()
    expect(parseChord('↓')).toBeNull()
    expect(parseChord('Ctrl+↑')).toBeNull()
  })
})

describe('matching a key press', () => {
  it('matches the exact chord', () => {
    const chord = parseChord('Ctrl+Shift+M')
    if (chord === null) throw new Error('Ctrl+Shift+M should parse')

    expect(matches(press('M', { ctrl: true, shift: true }), chord)).toBe(true)
    expect(matches(press('m', { ctrl: true, shift: true }), chord)).toBe(true)
  })

  it('does not fire when a modifier is missing', () => {
    // Ctrl+Shift+M must not fire on Ctrl+M. A shortcut that triggers on a chord the user did
    // not press is worse than one that never triggers, because they cannot tell what they did.
    const chord = parseChord('Ctrl+Shift+M')
    if (chord === null) throw new Error('Ctrl+Shift+M should parse')

    expect(matches(press('m', { ctrl: true }), chord)).toBe(false)
  })

  it('does not fire when an extra modifier is held', () => {
    // The reason every modifier is compared rather than only the wanted ones: Alt+Ctrl+L and
    // Ctrl+L are different shortcuts, and both exist.
    const flag = parseChord('Ctrl+L')
    const rules = parseChord('Alt+Ctrl+L')
    if (flag === null || rules === null) throw new Error('both should parse')

    expect(matches(press('l', { ctrl: true, alt: true }), flag)).toBe(false)
    expect(matches(press('l', { ctrl: true, alt: true }), rules)).toBe(true)
    expect(matches(press('l', { ctrl: true }), rules)).toBe(false)
  })

  it('separates Delete from Shift+Delete', () => {
    // One moves to Trash and the other destroys the message. Confusing them is not recoverable.
    const trash = parseChord('Delete')
    const permanent = parseChord('Shift+Delete')
    if (trash === null || permanent === null) throw new Error('both should parse')

    expect(matches(press('Delete'), trash)).toBe(true)
    expect(matches(press('Delete', { shift: true }), trash)).toBe(false)
    expect(matches(press('Delete', { shift: true }), permanent)).toBe(true)
  })
})

describe('the Phase 10 exit gate', () => {
  /**
   * docs/06: "complete a full triage session (read, flag, archive, reply, search, send)
   * without touching the mouse".
   *
   * The six verbs are named in the gate, so they are asserted rather than described. A
   * registry that lost one of these would still look complete — there are twenty-seven
   * entries — and the loss would only show up as somebody reaching for the mouse.
   */
  const TRIAGE: { verb: string; id: ShortcutId }[] = [
    { verb: 'read', id: 'toggleRead' },
    { verb: 'flag', id: 'flag' },
    { verb: 'archive', id: 'archive' },
    { verb: 'reply', id: 'reply' },
    { verb: 'search', id: 'search' },
    { verb: 'send', id: 'send' },
  ]

  it.each(TRIAGE)('can $verb from the keyboard', ({ id }) => {
    const shortcut = SHORTCUTS.find((entry) => entry.id === id)

    expect(shortcut, `no shortcut registered for ${id}`).toBeDefined()
    expect(shortcut?.keys).toBeTruthy()
  })

  it('reaches a message without a pointer', () => {
    // Reading presupposes selecting, and selecting is arrow keys inside the list. These are
    // `local` — owned by the focused control rather than bound globally — which is why they
    // would be missed by a check that only looked at what useShortcuts binds.
    for (const id of ['nextMessage', 'previousMessage'] as ShortcutId[]) {
      expect(SHORTCUTS.find((entry) => entry.id === id)).toBeDefined()
    }
  })

  it('lists every shortcut in the reference sheet', () => {
    // The sheet renders from this array, so the only way to have an unlisted shortcut is to
    // bind one somewhere else. Asserting the groups are all known catches an entry added with
    // a group the sheet does not render, which would make it invisible in Help.
    for (const shortcut of SHORTCUTS) {
      expect(GROUP_ORDER, `${shortcut.id} is in an ungrouped section`).toContain(shortcut.group)
    }
  })
})
