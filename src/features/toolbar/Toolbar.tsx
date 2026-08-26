import {
  Archive,
  Flag,
  Forward,
  FolderInput,
  PenSquare,
  Reply,
  ReplyAll,
  Search,
  ShieldAlert,
  Trash2,
} from 'lucide-react'

import { useMailStore } from '@/store/mail'
import { IconButton, TextField, Tooltip, TooltipGroup } from '@/ui'

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
export function Toolbar() {
  const selectedMessageIds = useMailStore((state) => state.selectedMessageIds)

  const hasSelection = selectedMessageIds.length > 0
  const single = selectedMessageIds.length === 1

  return (
    <div className={styles.toolbar} data-tauri-drag-region>
      <TooltipGroup>
        <div className={styles.group}>
          <Tooltip
            content="New Message"
            trigger={<IconButton icon={PenSquare} label="New Message" />}
          />
        </div>

        <div className={styles.group}>
          <Tooltip
            content="Reply"
            trigger={<IconButton icon={Reply} label="Reply" disabled={!single} />}
          />
          <Tooltip
            content="Reply All"
            trigger={<IconButton icon={ReplyAll} label="Reply All" disabled={!single} />}
          />
          <Tooltip
            content="Forward"
            trigger={<IconButton icon={Forward} label="Forward" disabled={!single} />}
          />
        </div>

        <div className={styles.group}>
          <Tooltip
            content="Archive"
            trigger={<IconButton icon={Archive} label="Archive" disabled={!hasSelection} />}
          />
          <Tooltip
            content="Delete"
            trigger={<IconButton icon={Trash2} label="Delete" disabled={!hasSelection} />}
          />
          <Tooltip
            content="Move to Junk"
            trigger={
              <IconButton icon={ShieldAlert} label="Move to Junk" disabled={!hasSelection} />
            }
          />
        </div>

        <div className={styles.group}>
          <Tooltip
            content="Move to…"
            trigger={<IconButton icon={FolderInput} label="Move to" disabled={!hasSelection} />}
          />
          <Tooltip
            content="Flag"
            trigger={<IconButton icon={Flag} label="Flag" disabled={!hasSelection} />}
          />
        </div>
      </TooltipGroup>

      <div className={styles.spacer} />

      <TextField
        label="Search"
        hideLabel
        variant="search"
        placeholder="Search"
        leadingIcon={Search}
        className={styles.search}
      />
    </div>
  )
}
