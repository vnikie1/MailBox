import { readFileSync } from 'node:fs'

import { describe, expect, it, vi, beforeEach } from 'vitest'

import { PANES, paneFrom } from '@/features/settings/panes'

/**
 * The Settings window. docs/06 Phase 11.
 *
 * Two things here are worth a test and neither is obvious from reading the components.
 *
 * The first is that four separate places name the panes — this table, the Rust command's guard,
 * the URL the window opens with, and the event that moves an already-open window. Three of those
 * are strings, and a mismatch does not fail to compile: Settings opens on General and the menu
 * item that asked for Rules looks like it did nothing.
 *
 * The second is the loop. The preferences the UI owns are announced to the other window, and a
 * window that re-announced what it had just been told would talk to its sibling for ever, each
 * one repainting the other. `applyRemote` exists to break that, and nothing about reading the
 * store makes it apparent that it must not broadcast.
 */

/** What `ipc/window.rs` will accept as a pane name. Kept identical to the guard there. */
function passesTheRustGuard(pane: string): boolean {
  // `is_ascii_lowercase()` on every char, which is what the Rust side does. Written as a
  // regex rather than a spread over the string, because spreading iterates code points and
  // the guard it mirrors iterates bytes — a difference that would matter the moment a pane
  // name contained anything outside ASCII.
  return /^[a-z]*$/.test(pane)
}

describe('the settings panes', () => {
  it('is the seven docs/06 Phase 11 asks for', () => {
    expect(PANES.map((pane) => pane.label)).toEqual([
      'General',
      'Accounts',
      'Composing',
      'Signatures',
      'Rules',
      'Privacy',
      'Advanced',
    ])
  })

  it('names each pane once', () => {
    const ids = PANES.map((pane) => pane.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('uses names the core will accept', () => {
    // The pane reaches Rust as a command argument and comes back in a query string. A name
    // with a `&` or a capital in it is refused there, and the window never opens.
    for (const pane of PANES) {
      expect(passesTheRustGuard(pane.id), `${pane.id} would be refused`).toBe(true)
    }
  })

  it('falls back to General rather than failing', () => {
    // A settings window that refuses to open because a query string was wrong is worse than
    // one that opens on the wrong pane — the wrong pane is one click from the right one.
    expect(paneFrom('rules')).toBe('rules')
    expect(paneFrom('nonsense')).toBe('general')
    expect(paneFrom(null)).toBe('general')
    expect(paneFrom(undefined)).toBe('general')
  })

  it('is the same list the Rust guard tests against', () => {
    // Rust cannot import this file, so it keeps its own copy. This is what notices when one
    // of the two is edited and the other is not.
    const rust = readFileSync('src-tauri/src/ipc/window.rs', 'utf8')
    const block = /const PANES: \[&str; \d+\] = \[([^\]]*)\]/.exec(rust)?.[1] ?? ''
    const named = [...block.matchAll(/"([a-z]+)"/g)].map((match) => match[1])

    expect(named).toEqual(PANES.map((pane) => pane.id))
  })
})

describe('display preferences across windows', () => {
  beforeEach(() => {
    vi.resetModules()
    localStorage.clear()
  })

  it('announces a change made here', async () => {
    const broadcast = vi.fn().mockResolvedValue(undefined)
    vi.doMock('@/lib/ipc', () => ({ broadcastDisplayPreferences: broadcast }))

    const { useSettingsStore } = await import('@/store/settings')
    useSettingsStore.getState().setTheme('dark')

    expect(broadcast).toHaveBeenCalledTimes(1)
    // The whole triple, not the one field that changed: the receiving window applies what it
    // is given, and a partial payload would reset the two it did not carry.
    expect(broadcast).toHaveBeenCalledWith({
      theme: 'dark',
      density: 'default',
      transparency: 'system',
    })
  })

  it('does not re-announce a change made elsewhere', async () => {
    // The loop. Without a separate entry point, window A tells B, B applies it and tells A,
    // A applies it and tells B, and neither window ever stops repainting.
    const broadcast = vi.fn().mockResolvedValue(undefined)
    vi.doMock('@/lib/ipc', () => ({ broadcastDisplayPreferences: broadcast }))

    const { useSettingsStore } = await import('@/store/settings')
    useSettingsStore
      .getState()
      .applyRemote({ theme: 'dark', density: 'compact', transparency: 'reduce' })

    expect(broadcast).not.toHaveBeenCalled()
    expect(useSettingsStore.getState().density).toBe('compact')
  })
})
