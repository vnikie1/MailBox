import {
  AlarmClock,
  Archive,
  Ban,
  CircleSlash,
  CornerUpLeft,
  FolderInput,
  Forward,
  MailOpen,
  Reply,
  ShieldAlert,
  Trash2,
  Wand2,
} from 'lucide-react'

import type { FlagName } from '@/lib/generated/FlagName'
import type { MessageRow } from '@/lib/generated/MessageRow'
import { RemindMenu } from '@/features/organise'
import { MenuItem, MenuSeparator, MenuSwatchRow } from '@/ui'

/**
 * The right-click menu on a message. docs/01 §5, and the order is macOS Mail's own.
 *
 * ## What is here and what is not
 *
 * Mail's menu has nineteen rows. This has fourteen, and the five that are missing are missing
 * because standing rule 18 forbids a menu item that does nothing:
 *
 *  - **Open** — there is no window that shows one stored message. The app opens three kinds of
 *    second window (compose, an `.eml` file, settings) and none of them is a reader.
 *  - **Send Again** — compose can only attach files from disk, and a stored message's
 *    attachments live inside its cached `.eml`. It would send, and quietly drop them.
 *  - **Forward as Attachment** — needs a fourth reply kind in the core and a `message/rfc822`
 *    row in the MIME table. Genuinely small; simply not written yet.
 *  - **Copy to** — there is no copy operation anywhere. The sync layer knows flag, move,
 *    delete and append-draft, and duplicating a message row touches the search index, the
 *    thread, the attachments and the undo stack.
 *  - **Unsubscribe** — the `List-Unsubscribe` header is never captured, and following one is a
 *    request that confirms the address is live, which is the same thing remote images are
 *    blocked by default for. A privacy decision, not menu wiring.
 *
 * ## Everything here goes through the shell's `actions`
 *
 * Not through IPC directly. That object already drives the toolbar and every keyboard
 * shortcut, so a third caller reaching past it would be a third place for the guards to drift
 * — the single-account rule on Move To, the single-selection rule on Reply, the toast on a
 * failed archive. The shortcut hints below are quoted from `app/shortcuts.ts` for the same
 * reason: one definition, three surfaces.
 */
export interface MessageContextMenuProps {
  /** The rows the menu acts on — the selection, or the row it was opened on. */
  rows: MessageRow[]
  /** The flag colours and their names, from the core. */
  flagNames: FlagName[]
  /** Addresses the user has blocked, so the row can offer the reverse. */
  blocked: ReadonlySet<string>
  actions: MessageMenuActions
}

export interface MessageMenuActions {
  reply: () => void
  replyAll: () => void
  forward: () => void
  redirect: () => void
  toggleRead: () => void
  markJunk: () => void
  delete: () => void
  archive: () => void
  moveTo: () => void
  runRules: () => void
  setFlag: (colour: string | null) => void
  blockSender: (address: string) => void
  unblockSender: (address: string) => void
}

export function MessageContextMenu({ rows, flagNames, blocked, actions }: MessageContextMenuProps) {
  const ids = rows.map((row) => row.id)
  const only = rows.length === 1 ? rows[0] : undefined

  // The same rule the core applies. `msg_toggle_read` decides the direction from the stored
  // rows — any unread means "mark them all read" — so computing the label the same way is what
  // stops the label and the outcome disagreeing on a selection where some are read and some
  // are not. Reading it off the rows the list already holds costs nothing.
  const anyUnread = rows.some((row) => !row.seen)

  // Undefined where the selection disagrees, which is what the swatch row wants: it then shows
  // no tick rather than claiming a colour none of them share.
  const first = rows[0]?.flagColor ?? null
  const sharedFlag = rows.every((row) => (row.flagColor ?? null) === first) ? first : undefined

  const sender = only?.fromAddr ?? null
  const isBlocked = sender !== null && blocked.has(sender.toLowerCase())

  return (
    <>
      <MenuItem
        label="Reply"
        icon={Reply}
        shortcut="Ctrl+R"
        disabled={only === undefined}
        onClick={actions.reply}
      />
      <MenuItem
        label="Reply All"
        icon={CornerUpLeft}
        shortcut="Ctrl+Shift+R"
        disabled={only === undefined}
        onClick={actions.replyAll}
      />
      <MenuItem
        label="Forward"
        icon={Forward}
        shortcut="Ctrl+Shift+F"
        disabled={only === undefined}
        onClick={actions.forward}
      />
      <MenuItem
        label="Redirect"
        icon={Forward}
        shortcut="Ctrl+Shift+E"
        disabled={only === undefined}
        onClick={actions.redirect}
      />

      <MenuSeparator />

      {/* A nested menu with no trigger renders as a submenu row with a chevron, which is what
          Mail draws here. `RemindMenu` is a finished component that until now was mounted
          nowhere at all. */}
      <RemindMenu ids={ids} icon={AlarmClock} />

      <MenuSeparator />

      <MenuItem
        label={anyUnread ? 'Mark as Read' : 'Mark as Unread'}
        icon={MailOpen}
        shortcut="Ctrl+U"
        onClick={actions.toggleRead}
      />
      <MenuItem
        label="Move to Junk"
        icon={ShieldAlert}
        shortcut="Ctrl+J"
        onClick={actions.markJunk}
      />
      <MenuItem
        label="Delete"
        icon={Trash2}
        shortcut="Delete"
        destructive
        onClick={actions.delete}
      />
      {/* Two-way, because the app can read the blocked list back. A one-way "Block Sender"
          that never says "Unblock" is a door that only opens one way. */}
      <MenuItem
        label={isBlocked ? 'Unblock Sender' : 'Block Sender'}
        icon={isBlocked ? CircleSlash : Ban}
        disabled={sender === null}
        onClick={() => {
          if (sender === null) return
          if (isBlocked) actions.unblockSender(sender)
          else actions.blockSender(sender)
        }}
      />

      <MenuSeparator />

      <MenuSwatchRow
        label="Flag:"
        options={flagNames.map((flag) => ({ value: flag.color, name: flag.name }))}
        current={sharedFlag}
        clearLabel="No Flag"
        onPick={actions.setFlag}
      />

      <MenuSeparator />

      <MenuItem label="Archive" icon={Archive} shortcut="Ctrl+Shift+A" onClick={actions.archive} />
      {/* An ellipsis rather than a chevron: this opens the typeahead sheet the shell already
          mounts, which carries the single-account rule and its own empty states. A submenu
          here would be a second picker with none of that. */}
      <MenuItem label="Move to…" icon={FolderInput} onClick={actions.moveTo} />

      <MenuSeparator />

      <MenuItem label="Apply Rules" icon={Wand2} shortcut="Alt+Ctrl+L" onClick={actions.runRules} />
    </>
  )
}
