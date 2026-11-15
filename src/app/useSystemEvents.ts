import { useEffect, useRef } from 'react'

import {
  composeOpen,
  handoffTake,
  onNotificationAction,
  onNotificationShow,
  type ToastAction,
} from '@/lib/ipc'
import { useArchiveMessages, useSetFlags } from '@/app/queries'
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
  const setFlags = useSetFlags()
  const selectMessage = useMailStore((state) => state.selectMessage)

  // The handlers are held in a ref so the effect below can subscribe once. Written as
  // dependencies instead, every mutation object identity change would tear down and rebuild
  // three OS listeners — and a toast pressed during that gap would do nothing at all.
  const handlers = useRef({ archive, setFlags, selectMessage })
  handlers.current = { archive, setFlags, selectMessage }

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
      const { archive: doArchive, setFlags: doSetFlags, selectMessage: select } = handlers.current

      switch (event.action) {
        case 'archive':
          doArchive.mutate([event.messageId])
          break

        case 'read':
          // Set, never toggled. This was the toggle, on the reasoning that a toast is only for
          // mail that has just arrived and so is unread. A toast outlives that — it sits in the
          // Action Center for hours, and the message is read on a phone meanwhile — and pressing
          // Mark as Read then marked it *unread*, everywhere (M33, 2026-11-14 audit).
          doSetFlags.mutate({ ids: [event.messageId], patch: { seen: true, flagged: null } })
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

    // Then whatever was pressed before this window could hear it: the toast that brought a
    // closed main window back is one (M30). Taken only once the listener is in place, so
    // nothing falls between the two. Not taken by a subscription already cancelled (Strict
    // Mode's first pass), and acted on once taken: the core has handed them over and will not
    // hand them over again.
    track(
      onNotificationAction(onAction).then((off) => {
        if (!cancelled) {
          void handoffTake<ToastAction>('notification:action').then((held) => {
            held.forEach(onAction)
          })
        }
        return off
      }),
    )

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
