import {
  Archive,
  Flag,
  Forward,
  FolderInput,
  PenSquare,
  Reply,
  ReplyAll,
  ShieldAlert,
  Trash2,
} from 'lucide-react'

import { useMailStore } from '@/store/mail'
import { SearchField } from '@/features/search'
import { IconButton, Tooltip, TooltipGroup } from '@/ui'

import styles from './Toolbar.module.css'

/**
 * The action toolbar. docs/02 §6.1, corrected against `assets/reference/`.
 *
 * Two departures from the spec, both because the macOS 26 reference disagrees with it:
 *
 *  - **Button order.** §6.1 lists sidebar-toggle | delete, archive, junk | reply,
 *    reply-all, forward | flag, move. Mail actually puts compose first, then the reply
 *    group, then the destructive group, then move and flag. Reply before delete matters:
 *    they are one pixel apart in the spec's order and the consequences are not symmetric.
 *  - **It is not one unified bar.** Each pane carries its own header at the same height,
 *    and Mica showing through all three is what makes them read as one band. This is the
 *    reader's header only: the sidebar toggle lives over the sidebar and the mailbox title
 *    over the list, which is where the reference puts them. Having it here as well, which
 *    the first version did, put two identical toggles on screen at once.
 *
 * Everything is disabled-looking rather than absent when nothing is selected, because a
 * toolbar whose buttons come and go changes width and shifts the search field — standing
 * rule 6.
 */
/**
 * What the toolbar buttons do.
 *
 * These are the same callbacks the keyboard shortcuts use, passed down from AppShell rather than
 * rebuilt here, so a button and its shortcut cannot drift apart.
 *
 * There were none of these until Phase 7's gate was attempted. Every button below rendered, took
 * a tooltip, greyed itself out correctly when nothing was selected -- and had no onClick. The
 * toolbar was decorative, which nobody noticed because the message list carries its own Reply,
 * Reply All and Forward buttons and the shortcuts all worked.
 */
export interface ToolbarActions {
  newMessage: () => void
  reply: () => void
  replyAll: () => void
  forward: () => void
  archive: () => void
  delete: () => void
  markJunk: () => void
  moveTo: () => void
  flag: () => void
}

export interface ToolbarProps {
  actions: ToolbarActions
  search: string
  onSearchChange: (text: string) => void
  onSearchCommit: (text: string) => void
}

export function Toolbar({ actions, search, onSearchChange, onSearchCommit }: ToolbarProps) {
  const selectedMessageIds = useMailStore((state) => state.selectedMessageIds)

  const hasSelection = selectedMessageIds.length > 0
  const single = selectedMessageIds.length === 1

  return (
    <div className={styles.toolbar} data-tauri-drag-region>
      <TooltipGroup>
        <div className={styles.group}>
          <Tooltip
            content="New Message"
            trigger={
              <IconButton icon={PenSquare} label="New Message" onClick={actions.newMessage} />
            }
          />
        </div>

        <div className={styles.group}>
          <Tooltip
            content="Reply"
            trigger={
              <IconButton icon={Reply} label="Reply" disabled={!single} onClick={actions.reply} />
            }
          />
          <Tooltip
            content="Reply All"
            trigger={
              <IconButton
                icon={ReplyAll}
                label="Reply All"
                disabled={!single}
                onClick={actions.replyAll}
              />
            }
          />
          <Tooltip
            content="Forward"
            trigger={
              <IconButton
                icon={Forward}
                label="Forward"
                disabled={!single}
                onClick={actions.forward}
              />
            }
          />
        </div>

        <div className={styles.group}>
          <Tooltip
            content="Archive"
            trigger={
              <IconButton
                icon={Archive}
                label="Archive"
                disabled={!hasSelection}
                onClick={actions.archive}
              />
            }
          />
          <Tooltip
            content="Delete"
            trigger={
              <IconButton
                icon={Trash2}
                label="Delete"
                disabled={!hasSelection}
                onClick={actions.delete}
              />
            }
          />
          <Tooltip
            content="Move to Junk"
            trigger={
              <IconButton
                icon={ShieldAlert}
                label="Move to Junk"
                disabled={!hasSelection}
                onClick={actions.markJunk}
              />
            }
          />
        </div>

        <div className={styles.group}>
          <Tooltip
            content="Move to…"
            trigger={
              <IconButton
                icon={FolderInput}
                label="Move to"
                disabled={!hasSelection}
                onClick={actions.moveTo}
              />
            }
          />
          <Tooltip
            content="Flag"
            trigger={
              <IconButton
                icon={Flag}
                label="Flag"
                disabled={!hasSelection}
                onClick={actions.flag}
              />
            }
          />
        </div>
      </TooltipGroup>

      <div className={styles.spacer} />

      <SearchField value={search} onChange={onSearchChange} onCommit={onSearchCommit} />
    </div>
  )
}
