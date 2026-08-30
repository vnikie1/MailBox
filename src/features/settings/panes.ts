import {
  AtSign,
  Bug,
  Filter,
  Lock,
  PenLine,
  Settings2,
  Signature,
  type LucideIcon,
} from 'lucide-react'

import type { SettingsPane } from '@/lib/ipc'

/**
 * The seven panes, in order. docs/06 Phase 11 names them.
 *
 * A table rather than a switch statement, for the reason the shortcut registry is a table:
 * the nav list, the window's routing and the Rust command's guard all have to agree on the
 * same set of names, and three hand-written lists drift. The Rust side has its own copy —
 * it cannot import this one — and a test on each side checks its copy against these names.
 */
export interface PaneDescriptor {
  id: SettingsPane
  label: string
  icon: LucideIcon
}

export const PANES: PaneDescriptor[] = [
  { id: 'general', label: 'General', icon: Settings2 },
  { id: 'accounts', label: 'Accounts', icon: AtSign },
  { id: 'composing', label: 'Composing', icon: PenLine },
  { id: 'signatures', label: 'Signatures', icon: Signature },
  { id: 'rules', label: 'Rules', icon: Filter },
  { id: 'privacy', label: 'Privacy', icon: Lock },
  { id: 'advanced', label: 'Advanced', icon: Bug },
]

/**
 * The pane a URL or an event asks for, or General.
 *
 * Falls back rather than throwing. A settings window that refuses to open because a query
 * string was wrong is a worse outcome than one that opens on the wrong pane, and the wrong
 * pane is one click from the right one.
 */
export function paneFrom(value: string | null | undefined): SettingsPane {
  const found = PANES.find((pane) => pane.id === value)
  return found?.id ?? 'general'
}
