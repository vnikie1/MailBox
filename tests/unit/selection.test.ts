import { beforeEach, describe, expect, it } from 'vitest'

import { useMailStore } from '@/store/mail'

/**
 * What the selection does when the message it is on leaves the list.
 *
 * Deleting a message is a **move to Trash**, so the row is still in the database — which is why
 * invalidating the reader's query could never fix this. The reader asked for the message again,
 * got it back, and went on rendering the mail the user had just deleted. The selection itself
 * has to move on.
 *
 * Written against "the selection is no longer in the list" rather than against deleting, so
 * archive, move, a rule filing something and a sync removing a message expunged on another
 * device all behave the same way.
 */
describe('the selection when a message leaves the list', () => {
  beforeEach(() => {
    useMailStore.setState({
      selectedMessageIds: [],
      anchorMessageId: null,
      focusedInThread: null,
    })
  })

  const select = (ids: number[]) => {
    useMailStore.setState({ selectedMessageIds: ids, anchorMessageId: ids[0] ?? null })
  }

  it('takes the row that slid into its place', () => {
    // The whole point: Delete, Delete, Delete without touching the mouse.
    select([20])
    useMailStore.getState().reconcileSelection([10, 30, 40], 1)

    expect(useMailStore.getState().selectedMessageIds).toEqual([30])
  })

  it('takes the last row when the one that went was at the end', () => {
    select([40])
    useMailStore.getState().reconcileSelection([10, 20, 30], 3)

    expect(useMailStore.getState().selectedMessageIds).toEqual([30])
  })

  it('leaves an untouched selection exactly alone', () => {
    // The ordinary case, and the one that must cost nothing: this runs on every list change.
    select([20])
    useMailStore.setState({ focusedInThread: 99 })
    useMailStore.getState().reconcileSelection([10, 20, 30], 1)

    expect(useMailStore.getState().selectedMessageIds).toEqual([20])
    expect(useMailStore.getState().focusedInThread).toBe(99)
  })

  it('keeps what survives of a multi-selection rather than jumping', () => {
    // The user still has a selection, and moving it would lose their place.
    select([10, 20, 30])
    useMailStore.getState().reconcileSelection([10, 30], 0)

    expect(useMailStore.getState().selectedMessageIds).toEqual([10, 30])
  })

  it('moves on when a whole multi-selection has gone', () => {
    select([20, 30])
    useMailStore.getState().reconcileSelection([10, 40, 50], 1)

    expect(useMailStore.getState().selectedMessageIds).toEqual([40])
  })

  it('clears when the list is empty', () => {
    // Deleting the last message in a mailbox. There is nothing to move to, and leaving the id
    // selected is what left the reader showing a message that is no longer here.
    select([20])
    useMailStore.getState().reconcileSelection([], 0)

    expect(useMailStore.getState().selectedMessageIds).toEqual([])
    expect(useMailStore.getState().anchorMessageId).toBeNull()
  })

  it('does nothing when nothing is selected', () => {
    useMailStore.getState().reconcileSelection([10, 20], 0)
    expect(useMailStore.getState().selectedMessageIds).toEqual([])
  })

  it('forgets the position inside the old conversation', () => {
    // Ctrl+↑/↓ move within the open thread. Carrying that position onto a different message
    // would put the reader somewhere the user never navigated to.
    select([20])
    useMailStore.setState({ focusedInThread: 21 })
    useMailStore.getState().reconcileSelection([10, 30], 1)

    expect(useMailStore.getState().selectedMessageIds).toEqual([30])
    expect(useMailStore.getState().focusedInThread).toBeNull()
  })
})
