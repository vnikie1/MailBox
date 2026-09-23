/**
 * Every keyboard shortcut, in one place. docs/01 §14, docs/06 Phase 10.
 *
 * ## Why a registry rather than handlers where they are used
 *
 * They started scattered — undo in the reader's hook, move and rules in the shell, arrow keys in
 * the list — and that arrangement has three failure modes, all of which had already happened by
 * the time this was written:
 *
 * * **Silent collisions.** Two handlers can claim the same chord and both run, or one swallows
 *   the other depending on which mounted first. Nothing tells you; the shortcut just behaves
 *   oddly. Here the table is the single source, and a test asserts no chord appears twice.
 * * **A Help sheet that lies.** A reference list written by hand drifts from the code within a
 *   week. This one *is* the code — the sheet renders the same array the handlers are built
 *   from, so it cannot describe a shortcut that does not exist or miss one that does.
 * * **Inconsistent guarding.** Every shortcut needs the same two answers: does it work while
 *   the caret is in a text field, and does it need a selection? Written separately, each
 *   handler answers them slightly differently.
 *
 * ## What is deliberately not handled here
 *
 * Arrow keys and Space stay with the list and the reader. They are *navigation within a focused
 * control*, not application commands: the list already tracks the visible order that Up and Down
 * move through, and a global handler would have to reach back into it for state it does not own.
 * They are listed in the reference sheet, because from the user's side they are shortcuts like
 * any other.
 */

/** What a chord needs to be true before it fires. */
export interface Requirement {
  /** Fires only when at least one message is selected. */
  selection?: boolean
}

export interface Shortcut {
  id: ShortcutId
  /** How it reads in the Help sheet, e.g. `Ctrl+Shift+M`. */
  keys: string
  label: string
  /** Which part of Help it appears under. */
  group: Group
  requires?: Requirement
  /**
   * True when the chord is owned by a focused control rather than dispatched globally.
   *
   * Listed for the user, never bound here. See the module note.
   */
  local?: boolean
}

export type Group = 'Message' | 'Compose' | 'Organise' | 'Navigate' | 'View'

export type ShortcutId =
  | 'newMessage'
  | 'send'
  | 'reply'
  | 'replyAll'
  | 'forward'
  | 'redirect'
  | 'archive'
  | 'delete'
  | 'deletePermanently'
  | 'toggleRead'
  | 'flag'
  | 'markJunk'
  | 'moveTo'
  | 'search'
  | 'getMail'
  | 'runRules'
  | 'undo'
  | 'redo'
  | 'toggleSidebar'
  | 'nextMessage'
  | 'previousMessage'
  | 'nextInThread'
  | 'previousInThread'
  | 'expandThread'
  | 'collapseThread'
  | 'previewAttachment'
  | 'jumpToMailbox'
  | 'moveFavourite'
  | 'showShortcuts'
  | 'settings'

/**
 * The table. Order is the order Help shows them within each group.
 *
 * Every row is docs/01 §14 unless marked otherwise. Two additions: Alt+Ctrl+L for rules, which
 * docs/01 §8 specifies, and Ctrl+Shift+Z for redo, which §14 omits — an undo with no redo is a
 * trap, since one keystroke too many cannot be taken back.
 */
export const SHORTCUTS: Shortcut[] = [
  { id: 'newMessage', keys: 'Ctrl+N', label: 'New message', group: 'Compose' },
  { id: 'send', keys: 'Ctrl+Enter', label: 'Send', group: 'Compose', local: true },

  { id: 'reply', keys: 'Ctrl+R', label: 'Reply', group: 'Message', requires: { selection: true } },
  {
    id: 'replyAll',
    keys: 'Ctrl+Shift+R',
    label: 'Reply all',
    group: 'Message',
    requires: { selection: true },
  },
  {
    id: 'forward',
    keys: 'Ctrl+Shift+F',
    label: 'Forward',
    group: 'Message',
    requires: { selection: true },
  },
  {
    id: 'redirect',
    keys: 'Ctrl+Shift+E',
    label: 'Redirect',
    group: 'Message',
    requires: { selection: true },
  },

  {
    id: 'archive',
    keys: 'Ctrl+Shift+A',
    label: 'Archive',
    group: 'Organise',
    requires: { selection: true },
  },
  {
    id: 'delete',
    keys: 'Delete',
    label: 'Move to Trash',
    group: 'Organise',
    requires: { selection: true },
  },
  {
    id: 'deletePermanently',
    keys: 'Shift+Delete',
    label: 'Delete permanently',
    group: 'Organise',
    requires: { selection: true },
  },
  {
    id: 'toggleRead',
    keys: 'Ctrl+U',
    label: 'Mark read or unread',
    group: 'Organise',
    requires: { selection: true },
  },
  { id: 'flag', keys: 'Ctrl+L', label: 'Flag', group: 'Organise', requires: { selection: true } },
  {
    id: 'markJunk',
    keys: 'Ctrl+J',
    label: 'Mark as junk',
    group: 'Organise',
    requires: { selection: true },
  },
  {
    id: 'moveTo',
    keys: 'Ctrl+Shift+M',
    label: 'Move to mailbox…',
    group: 'Organise',
    requires: { selection: true },
  },
  {
    id: 'runRules',
    keys: 'Alt+Ctrl+L',
    label: 'Apply rules',
    group: 'Organise',
    requires: { selection: true },
  },
  { id: 'undo', keys: 'Ctrl+Z', label: 'Undo', group: 'Organise' },
  { id: 'redo', keys: 'Ctrl+Shift+Z', label: 'Redo', group: 'Organise' },

  { id: 'search', keys: 'Ctrl+F', label: 'Search', group: 'Navigate' },
  { id: 'getMail', keys: 'F5', label: 'Get new mail', group: 'Navigate' },
  { id: 'jumpToMailbox', keys: 'Ctrl+1–9', label: 'Jump to mailbox', group: 'Navigate' },
  // The keyboard's way to do what a drag in Favourites does, on the focused row; the sidebar
  // owns it, as it owns the arrows that move between rows.
  {
    id: 'moveFavourite',
    keys: 'Alt+↑ / Alt+↓',
    label: 'Move a favourite up or down',
    group: 'Navigate',
    local: true,
  },
  { id: 'nextMessage', keys: '↓', label: 'Next message', group: 'Navigate', local: true },
  { id: 'previousMessage', keys: '↑', label: 'Previous message', group: 'Navigate', local: true },
  { id: 'nextInThread', keys: 'Ctrl+↓', label: 'Next in thread', group: 'Navigate' },
  { id: 'previousInThread', keys: 'Ctrl+↑', label: 'Previous in thread', group: 'Navigate' },

  { id: 'toggleSidebar', keys: 'Ctrl+Shift+S', label: 'Show or hide the sidebar', group: 'View' },
  { id: 'showShortcuts', keys: 'F1', label: 'Keyboard shortcuts', group: 'View' },
  { id: 'settings', keys: 'Ctrl+,', label: 'Settings', group: 'View' },
  { id: 'expandThread', keys: '→', label: 'Expand thread', group: 'View', local: true },
  { id: 'collapseThread', keys: '←', label: 'Collapse thread', group: 'View', local: true },
  {
    id: 'previewAttachment',
    keys: 'Space',
    label: 'Preview attachment',
    group: 'View',
    local: true,
  },
]

export const GROUP_ORDER: Group[] = ['Message', 'Compose', 'Organise', 'Navigate', 'View']

/** A chord as this module compares them: modifiers in a fixed order, then the key. */
export interface Chord {
  ctrl: boolean
  shift: boolean
  alt: boolean
  key: string
}

/** The glyphs the Help sheet shows, and the `KeyboardEvent.key` each one arrives as. */
const ARROW_KEYS: Record<string, string> = {
  '↓': 'arrowdown',
  '↑': 'arrowup',
  '→': 'arrowright',
  '←': 'arrowleft',
}

/**
 * Parses `Ctrl+Shift+M` into something a key event can be compared against.
 *
 * Returns `null` for a row that is not one chord: `Ctrl+1–9` is a range, and a **bare** arrow
 * belongs to whichever list has focus rather than to the application.
 *
 * A *modified* arrow is not either of those. `Ctrl+↓` is an ordinary chord that happens to be
 * drawn with a glyph, and rejecting it along with the bare arrows is why "next in thread" and
 * "previous in thread" appeared in the Help sheet for three phases while the dispatcher skipped
 * their rows entirely and no handler was ever written for them.
 */
export function parseChord(keys: string): Chord | null {
  const parts = keys.split('+').map((part) => part.trim())
  const last = parts[parts.length - 1] ?? ''
  const key = last.toLowerCase()

  // A range, or anything else that is not one literal key.
  if (key === '' || key.includes('–')) return null

  const ctrl = parts.includes('Ctrl')
  const shift = parts.includes('Shift')
  const alt = parts.includes('Alt')

  const arrow = ARROW_KEYS[last]
  if (arrow !== undefined) {
    // Unmodified, it is the focused control's key and not ours.
    if (!ctrl && !shift && !alt) return null
    return { ctrl, shift, alt, key: arrow }
  }

  return { ctrl, shift, alt, key }
}

/** Whether a keyboard event is this chord. */
export function matches(event: KeyboardEvent, chord: Chord): boolean {
  // Every modifier is compared, including the ones the chord does not want. Without that,
  // Ctrl+Shift+M would also fire on Ctrl+M — and a shortcut that triggers on a chord the user
  // did not press is worse than one that does not trigger at all.
  return (
    event.ctrlKey === chord.ctrl &&
    event.shiftKey === chord.shift &&
    event.altKey === chord.alt &&
    event.key.toLowerCase() === chord.key
  )
}
