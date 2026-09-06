import { describe, expect, it } from 'vitest'

import {
  canDropInMailbox,
  draggedMessageIds,
  startMessageDrag,
  type DraggedMessage,
} from '@/lib/messageDrag'

/**
 * The rules that decide where dragged mail may land.
 *
 * Tested against a stand-in rather than a real `DataTransfer`, because the interesting case
 * cannot be reproduced with one: during `dragover` the browser returns an empty string from
 * `getData` and only `types` is readable, and jsdom models neither that restriction nor the
 * drag at all. The stand-in below implements exactly the three members the module uses, and
 * `duringDragover` reproduces the restriction that shapes the whole design — see
 * `src/lib/messageDrag.ts`.
 */
function transfer(): DataTransfer {
  const data = new Map<string, string>()

  return {
    effectAllowed: 'none',
    get types() {
      return [...data.keys()]
    },
    setData(type: string, value: string) {
      data.set(type.toLowerCase(), value)
    },
    getData(type: string) {
      return data.get(type.toLowerCase()) ?? ''
    },
  } as unknown as DataTransfer
}

/**
 * The same drag as the browser presents it *before* the drop: types intact, data withheld.
 * A rule that needs the data cannot be enforced here, which is the point.
 */
function duringDragover(full: DataTransfer): DataTransfer {
  return {
    types: full.types,
    setData: () => undefined,
    getData: () => '',
    effectAllowed: full.effectAllowed,
  } as unknown as DataTransfer
}

const NORTHGATE = 1
const ICLOUD = 2

const inbox = { id: 10, accountId: NORTHGATE }
const clients = { id: 11, accountId: NORTHGATE }
const family = { id: 20, accountId: ICLOUD }

function message(id: number, accountId: number, mailboxId: number): DraggedMessage {
  return { id, accountId, mailboxId }
}

describe('dragging messages onto a mailbox', () => {
  it('carries the ids through to the drop', () => {
    const dt = transfer()
    startMessageDrag(dt, [message(1, NORTHGATE, inbox.id), message(2, NORTHGATE, inbox.id)])

    expect(draggedMessageIds(dt)).toEqual([1, 2])
    expect(dt.effectAllowed).toBe('move')
  })

  it('accepts another folder in the same account', () => {
    const dt = transfer()
    startMessageDrag(dt, [message(1, NORTHGATE, inbox.id)])

    expect(canDropInMailbox(duringDragover(dt), clients)).toBe(true)
  })

  it('refuses a folder in another account', () => {
    // The core refuses this too, with `crossAccount`. Refusing it here is what puts the
    // no-drop cursor under the pointer instead of a toast after the fact.
    const dt = transfer()
    startMessageDrag(dt, [message(1, NORTHGATE, inbox.id)])

    expect(canDropInMailbox(duringDragover(dt), family)).toBe(false)
  })

  it('refuses the mailbox the messages are already in', () => {
    const dt = transfer()
    startMessageDrag(dt, [message(1, NORTHGATE, inbox.id)])

    expect(canDropInMailbox(duringDragover(dt), inbox)).toBe(false)
  })

  it('refuses everywhere when the selection spans two accounts', () => {
    // A unified mailbox makes this easy to do by accident: All Inboxes lists every account
    // at once, so Ctrl-clicking two rows can pick up mail that has no common destination.
    const dt = transfer()
    startMessageDrag(dt, [message(1, NORTHGATE, inbox.id), message(2, ICLOUD, family.id)])

    expect(canDropInMailbox(duringDragover(dt), clients)).toBe(false)
    expect(canDropInMailbox(duringDragover(dt), family)).toBe(false)
    expect(canDropInMailbox(duringDragover(dt), inbox)).toBe(false)
  })

  it('still accepts a destination when the selection spans two mailboxes of one account', () => {
    // Only the *account* has to agree. Two folders' worth of mail from one account has a
    // perfectly good destination, and the origin rule must not swallow it.
    const dt = transfer()
    startMessageDrag(dt, [message(1, NORTHGATE, inbox.id), message(2, NORTHGATE, clients.id)])

    expect(canDropInMailbox(duringDragover(dt), { id: 12, accountId: NORTHGATE })).toBe(true)
  })

  it('ignores a drag that is not messages', () => {
    const dt = transfer()
    dt.setData('text/plain', 'some text from another application')

    expect(canDropInMailbox(duringDragover(dt), clients)).toBe(false)
    expect(draggedMessageIds(dt)).toEqual([])
  })

  it('drops nothing when the payload is malformed', () => {
    const dt = transfer()
    dt.setData('application/x-mailbox-threads', 'not-a-number  -3 0 7')

    expect(draggedMessageIds(dt)).toEqual([7])
  })
})
