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
    // Reference-sheet entries, not bindings: a range, and a bare arrow, which belongs to
    // whichever list has focus.
    expect(parseChord('Ctrl+1–9')).toBeNull()
    expect(parseChord('↓')).toBeNull()
    expect(parseChord('→')).toBeNull()
  })

  it('reads a modified arrow as an ordinary chord', () => {
    // A bare arrow is the focused control's. `Ctrl+↓` is not — it is a chord that happens to be
    // drawn with a glyph, and refusing it along with the bare ones is why "next in thread" and
    // "previous in thread" sat in the Help sheet for three phases with no handler behind them:
    // the dispatcher skipped every row this returned null for.
    expect(parseChord('Ctrl+↓')).toEqual({
      ctrl: true,
      shift: false,
      alt: false,
      key: 'arrowdown',
    })
    expect(parseChord('Ctrl+↑')).toEqual({ ctrl: true, shift: false, alt: false, key: 'arrowup' })
  })

  it('matches a modified arrow against the event a browser reports', () => {
    // The glyph is for the Help sheet; `KeyboardEvent.key` says "ArrowDown". A chord that
    // parses and then never matches is the same bug one layer along.
    const chord = parseChord('Ctrl+↓')
    if (chord === null) throw new Error('Ctrl+↓ should parse')

    expect(matches(press('ArrowDown', { ctrl: true }), chord)).toBe(true)
    expect(matches(press('ArrowDown'), chord)).toBe(false)
    expect(matches(press('ArrowUp', { ctrl: true }), chord)).toBe(false)
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

describe('one listener, one table', () => {
  it('registers no window keydown handler outside the dispatcher', async () => {
    // `binds no chord twice` above checks the registry. It cannot see a chord bound somewhere
    // else entirely, and one was: `useUndo` kept its own `window` keydown listener for Ctrl+Z
    // alongside the table's. Both fired — separate listeners on the same target, and
    // `preventDefault` does not stop a sibling — so one keypress ran undo twice and took back
    // two actions.
    //
    // It hid behind the shape of the stack: with a single step, the second call found nothing
    // and did nothing, which is exactly the case anyone testing by hand tries first.
    // Namespaces rather than destructured methods: pulling a method off a module loses its
    // binding as far as the linter is concerned, and it is right to say so in general.
    const files = await import('node:fs/promises')
    const paths = await import('node:path')

    const walk = async (dir: string): Promise<string[]> => {
      const entries = await files.readdir(dir, { withFileTypes: true })
      const found: string[] = []

      for (const entry of entries) {
        const path = paths.join(dir, entry.name)
        if (entry.isDirectory()) found.push(...(await walk(path)))
        else if (/\.tsx?$/.test(entry.name)) found.push(path)
      }

      return found
    }

    const offenders: string[] = []

    for (const file of await walk('src')) {
      if (file.includes('useShortcuts')) continue

      const source = await files.readFile(file, 'utf8')

      // Comments stripped first. This reported ComposeWindow the moment a comment there
      // explained *why* it uses a React handler rather than a window listener: prose about
      // a call is not a call, and a test whose first result is a false alarm teaches whoever
      // sees the second one to ignore it.
      const code = source.replace(/^\s*(\/\/|\*).*$/gm, '')

      if (/addEventListener\(\s*['"]keydown['"]/.test(code)) offenders.push(file)
    }

    expect(
      offenders,
      `these files bind keys outside the shortcut table, so the table, the Help sheet and the ` +
        `dispatcher no longer agree — and a chord bound in two places fires twice`,
    ).toEqual([])
  })
})

/**
 * Shortcuts the Help sheet lists that nothing binds.
 *
 * The sheet is rendered from the same table the dispatcher reads, so a row with no handler is
 * a promise the app does not keep. `Handlers` is a `Partial<Record<...>>`, which is what let
 * this happen quietly: omitting one is not a type error.
 */
describe('every advertised shortcut is bound', () => {
  /**
   * Rows that are listed and deliberately not implemented, with the reason.
   *
   * Empty, and worth keeping empty. It held Ctrl+↑ and Ctrl+↓ — "next and previous in the open
   * conversation" — on the grounds that the reader had no notion of a position inside a thread
   * and faking one would have done nothing visible. It has one now, so they are bound and the
   * list is bare again.
   *
   * An entry here is a debt that is written down. The test below proves the list cannot quietly
   * become a place where a working shortcut hides from the one above it.
   */
  const KNOWN_GAPS: ShortcutId[] = []

  it('gives each non-local shortcut a handler in the shell', async () => {
    const files = await import('node:fs/promises')
    const shell = await files.readFile('src/features/shell/AppShell.tsx', 'utf8')

    // The handler object literal only, so a mention in a comment or an import does not count.
    const start = shell.indexOf('const actions = {')
    expect(start, 'the handler map in AppShell has been renamed').toBeGreaterThan(-1)
    const actions = shell.slice(start, shell.indexOf('\n  useShortcuts(', start))

    // Property names at the start of a line: `archive: useCallback(` and the `undo,` shorthand.
    const bound = new Set(
      actions
        .split('\n')
        .map((line) => /^\s*([a-zA-Z]+)\s*[:,]/.exec(line)?.[1])
        .filter((name): name is string => name !== undefined),
    )

    const unbound = SHORTCUTS.filter((shortcut) => shortcut.local !== true)
      .filter((shortcut) => !KNOWN_GAPS.includes(shortcut.id))
      .filter((shortcut) => !bound.has(shortcut.id))
      .map((shortcut) => `${shortcut.id} (${shortcut.keys}, ${shortcut.label})`)

    expect(
      unbound,
      'these are listed in the Help sheet and have no handler, so pressing them does nothing',
    ).toEqual([])
  })

  it('special-cases the chords the table cannot express', async () => {
    // `parseChord` returns null for a range or an arrow, so those rows never reach the loop.
    // That is deliberate, but it means the dispatcher has to handle them by hand — and a row
    // that parses to null with nothing special-casing it is invisible in both places. Ctrl+1–9
    // sat like that: skipped by the table, no handler written, listed in Help throughout.
    const unparseable = SHORTCUTS.filter(
      (shortcut) => shortcut.local !== true && parseChord(shortcut.keys) === null,
    ).filter((shortcut) => !KNOWN_GAPS.includes(shortcut.id))

    expect(unparseable.map((shortcut) => shortcut.id)).toEqual(['jumpToMailbox'])

    const files = await import('node:fs/promises')
    const dispatcher = await files.readFile('src/app/useShortcuts.ts', 'utf8')

    for (const shortcut of unparseable) {
      expect(
        dispatcher.includes(shortcut.id),
        `${shortcut.keys} cannot be parsed from the table and is not special-cased in the ` +
          `dispatcher either, so it is advertised in Help and unreachable`,
      ).toBe(true)
    }
  })

  it('keeps the known gaps honest', () => {
    // A gap that has since been filled should be removed from the list, or the list becomes a
    // place where a working shortcut hides from the test above.
    for (const id of KNOWN_GAPS) {
      expect(
        SHORTCUTS.some((shortcut) => shortcut.id === id),
        `${id} is listed as a known gap but is no longer in the table`,
      ).toBe(true)
    }
  })
})
