import { describe, expect, it } from 'vitest'

import { buildSidebar } from '@/features/sidebar/model'
import { fromFlat, toFlat } from '@/features/organise'
import type { AccountRow } from '@/lib/generated/AccountRow'
import type { Condition } from '@/lib/generated/Condition'
import type { MailboxRow } from '@/lib/generated/MailboxRow'
import type { Predicate } from '@/lib/generated/Predicate'
import type { SmartMailbox } from '@/lib/generated/SmartMailbox'

/**
 * The pure half of Phase 8's UI. docs/01 §8.
 *
 * The predicate round-trip is the part worth testing hard: it is the only place in the app
 * where a user's saved intent is decoded, shown, and re-encoded, and a lossy trip there means
 * a rule that changes meaning when someone opens it to look.
 */

const CONDITION: Condition = { field: 'from', op: 'contains', value: 'ada@example.test' }

describe('predicate round-trip', () => {
  it('survives a flat predicate unchanged', () => {
    const original = fromFlat(true, [CONDITION])
    const flat = toFlat(original)

    // Narrowed rather than asserted, so a regression that returns null fails here with a
    // readable message instead of throwing on a `!`.
    if (flat === null) throw new Error('a flat predicate did not round-trip')

    expect(flat.matchAll).toBe(true)
    expect(flat.conditions).toEqual([CONDITION])
    expect(fromFlat(flat.matchAll, flat.conditions)).toEqual(original)
  })

  it('keeps all and any apart', () => {
    expect(toFlat(fromFlat(false, [CONDITION]))?.matchAll).toBe(false)
    expect(toFlat(fromFlat(true, [CONDITION]))?.matchAll).toBe(true)
  })

  it('reads a bare condition as a one-row list', () => {
    // What the core stores for a smart mailbox with exactly one test. Refusing it would make
    // a perfectly ordinary saved search uneditable.
    const bare: Predicate = { type: 'is', value: CONDITION }
    expect(toFlat(bare)?.conditions).toEqual([CONDITION])
  })

  it('refuses a nested predicate rather than flattening it', () => {
    // This is the important one. Flattening would save back something meaning something
    // different from what the user opened — and for a rule that files mail unattended, a
    // silent change of meaning is the worst outcome there is.
    const nested: Predicate = {
      type: 'all',
      value: [
        { type: 'is', value: CONDITION },
        { type: 'any', value: [{ type: 'is', value: CONDITION }] },
      ],
    }

    expect(toFlat(nested)).toBeNull()
  })

  it('refuses a negation, which the flat editor cannot express', () => {
    const negated: Predicate = { type: 'not', value: { type: 'is', value: CONDITION } }
    expect(toFlat(negated)).toBeNull()
  })

  it('reads an empty group as an empty list rather than failing', () => {
    // The core treats an empty `all` as "everything" and an empty `any` as "nothing". Neither
    // is a parse failure, so the editor should show the group rather than lock it.
    expect(toFlat({ type: 'all', value: [] })).toEqual({ matchAll: true, conditions: [] })
    expect(toFlat({ type: 'any', value: [] })).toEqual({ matchAll: false, conditions: [] })
  })
})

const ACCOUNT: AccountRow = {
  id: 1,
  displayName: 'Test',
  email: 'me@example.test',
  provider: 'other',
}

const INBOX: MailboxRow = {
  id: 1,
  accountId: 1,
  parentId: null,
  displayName: 'Inbox',
  role: 'inbox',
  unreadCount: 3,
  totalCount: 10,
}

describe('sidebar', () => {
  it('puts smart mailboxes in their own section, carrying their predicate', () => {
    const smart: SmartMailbox = {
      id: 7,
      name: 'Unread from Ada',
      icon: null,
      predicate: fromFlat(true, [CONDITION]),
      sortOrder: 0,
    }

    const sections = buildSidebar([ACCOUNT], [INBOX], [smart], [])
    const section = sections.find((entry) => entry.id === 'smart')

    expect(section?.nodes).toHaveLength(1)
    expect(section?.nodes[0]?.label).toBe('Unread from Ada')
    expect(section?.nodes[0]?.predicate).toEqual(smart.predicate)
    // A saved search is not a folder. Carrying a mailbox id as well would leave two ways to
    // ask the same question and no rule about which wins.
    expect(section?.nodes[0]?.mailboxIds).toEqual([])
  })

  it('gives Flagged one child per named colour', () => {
    const sections = buildSidebar(
      [ACCOUNT],
      [INBOX],
      [],
      [
        { color: 'red', name: 'Urgent' },
        { color: 'blue', name: 'Blue' },
      ],
    )

    const flagged = sections
      .find((entry) => entry.id === 'favourites')
      ?.nodes.find((node) => node.id === 'flagged')

    expect(flagged?.predicate).toBeDefined()
    expect(flagged?.children.map((child) => child.label)).toEqual(['Urgent', 'Blue'])
    // The renamed one keeps its colour key, so the predicate still finds the right mail even
    // though the label no longer says "Red".
    expect(flagged?.children[0]?.id).toBe('flag-red')
  })

  it('still builds when there are no smart mailboxes or flag names', () => {
    // The state of a fresh install, and the state of the browser gallery, where the core
    // cannot answer at all.
    const sections = buildSidebar([ACCOUNT], [INBOX])

    expect(sections.find((entry) => entry.id === 'smart')?.nodes).toEqual([])
    expect(
      sections.find((entry) => entry.id === 'favourites')?.nodes.find((n) => n.id === 'flagged')
        ?.children,
    ).toEqual([])
  })
})

describe('VIP mailbox', () => {
  it('appears only once there is a VIP', () => {
    // An empty row that can never fill reads as a broken feature rather than an unused one.
    const without = buildSidebar([ACCOUNT], [INBOX], [], [], [])
    expect(without.find((s) => s.id === 'favourites')?.nodes.some((n) => n.id === 'vips')).toBe(
      false,
    )

    const withVip = buildSidebar(
      [ACCOUNT],
      [INBOX],
      [],
      [],
      [{ address: 'ada@example.test', addedAt: 0 }],
    )
    expect(withVip.find((s) => s.id === 'favourites')?.nodes.some((n) => n.id === 'vips')).toBe(
      true,
    )
  })

  it('matches on the sender rather than anywhere in the text', () => {
    // "From these people", not "mentions these people" — otherwise a newsletter quoting a VIP's
    // address lands in the row meant for mail they actually sent.
    const sections = buildSidebar(
      [ACCOUNT],
      [INBOX],
      [],
      [],
      [
        { address: 'ada@example.test', addedAt: 0 },
        { address: 'grace@example.test', addedAt: 0 },
      ],
    )

    const vips = sections.find((s) => s.id === 'favourites')?.nodes.find((n) => n.id === 'vips')
    const predicate = vips?.predicate

    expect(predicate?.type).toBe('any')
    if (predicate?.type !== 'any') throw new Error('expected an any group')

    expect(predicate.value).toHaveLength(2)
    for (const child of predicate.value) {
      expect(child.type).toBe('is')
      if (child.type !== 'is') throw new Error('expected a condition')
      expect(child.value.field).toBe('from')
    }
  })

  it('is a saved search, not a folder', () => {
    const sections = buildSidebar(
      [ACCOUNT],
      [INBOX],
      [],
      [],
      [{ address: 'ada@example.test', addedAt: 0 }],
    )
    const vips = sections.find((s) => s.id === 'favourites')?.nodes.find((n) => n.id === 'vips')

    expect(vips?.mailboxIds).toEqual([])
  })
})
