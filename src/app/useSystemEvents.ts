import { useEffect, useRef } from 'react'

import { composeOpen, onNotificationAction, onNotificationShow, type ToastAction } from '@/lib/ipc'
import { useArchiveMessages, useToggleRead } from '@/app/queries'
import { useMailStore } from '@/store/mail'

/**
 * Everything the operating system asks the app to do. docs/06 Phase 10.
 *
 * Three sources, one hook, because they all arrive the same way — an event from the core naming
 * something the user did *outside* the window — and all three end in the same place: a message
 * selected, or a compose window open.
 *
 * - **Toast buttons.** Reply, Archive, Mark as Read, and the toast body itself.
 * - **The summary toast**, which has nothing to act on and only brings the window forward.
 *
 * `mailto:` links are deliberately *not* here. They open a compose window straight from Rust,
 * because the sanitised fields have to reach that window and routing them through this side
 * would mean parsing the link a second time in TypeScript — and the second parser is the one
 * that forgets to drop Bcc. See platform/links.rs.
 *
 * ## Why the actions land here rather than in Rust
 *
 * Archive and Mark as Read both exist already, as mutations, with the cache invalidation that
 * makes the message list actually update. Reimplementing them behind the toast would produce a
 * second archive that works on the server and leaves the list showing a message that is no
 * longer there. So the Rust side decides *that* something was pressed and this side decides what
 * that means, which is the same split the rest of the app uses.
 */
export function useSystemEvents() {
  const archive = useArchiveMessages()
  const toggleRead = useToggleRead()
  const selectMessage = useMailStore((state) => state.selectMessage)

  // The handlers are held in a ref so the effect below can subscribe once. Written as
  // dependencies instead, every mutation object identity change would tear down and rebuild
  // three OS listeners — and a toast pressed during that gap would do nothing at all.
  const handlers = useRef({ archive, toggleRead, selectMessage })
  handlers.current = { archive, toggleRead, selectMessage }

  useEffect(() => {
    let cancelled = false
    const unlisteners: (() => void)[] = []

    const track = (promise: Promise<() => void>) => {
      void promise.then((off) => {
        if (cancelled) off()
        else unlisteners.push(off)
      })
    }

    const onAction = (event: ToastAction) => {
      const {
        archive: doArchive,
        toggleRead: doToggleRead,
        selectMessage: select,
      } = handlers.current

      switch (event.action) {
        case 'archive':
          doArchive.mutate([event.messageId])
          break

        case 'read':
          // Deliberately the toggle rather than a "mark read" of its own. The button only
          // appears on a toast for mail that just arrived, so the message is unread by
          // construction and the toggle can only move it one way.
          doToggleRead.mutate([event.messageId])
          break

        case 'reply':
          void composeOpen(event.messageId, 'reply')
          break

        case 'open':
          // Selecting it is what "open" means in a three-pane client. Bringing the window
          // forward is already done, in Rust, before this event was emitted.
          select(event.messageId)
          break
      }
    }

    track(onNotificationAction(onAction))

    // The summary toast has no action of its own: Rust brings the window forward on activation
    // and there is nothing further to do here. Subscribed anyway so that the day it grows one,
    // the place it goes already exists.
    track(onNotificationShow(() => undefined))

    return () => {
      cancelled = true
      unlisteners.forEach((off) => {
        off()
      })
    }
  }, [])
}
