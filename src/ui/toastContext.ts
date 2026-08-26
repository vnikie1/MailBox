import { createContext, use } from 'react'
import type { LucideIcon } from 'lucide-react'

/**
 * Kept out of Toast.tsx so that file exports only components — the react-refresh rule is
 * an error in this project, and a module mixing components with hooks loses fast refresh
 * for everything in it.
 */

export interface ToastAction {
  label: string
  onAction: () => void
}

export interface ToastOptions {
  title: string
  description?: string
  icon?: LucideIcon
  /**
   * A single action, and in practice it is nearly always Undo. docs/01 §14 requires
   * Ctrl+Z to undo delete, move, archive and send, and a toast is where that offer
   * becomes visible.
   */
  action?: ToastAction
  /** Milliseconds on screen. Defaults to the --toast-dwell token. */
  duration?: number
}

export interface ToastApi {
  /** Shows a toast and returns its id, so a caller can dismiss it early. */
  show: (options: ToastOptions) => string
  dismiss: (id: string) => void
}

export const ToastContext = createContext<ToastApi | null>(null)

export function useToast(): ToastApi {
  const api = use(ToastContext)
  if (!api) throw new Error('useToast must be used inside a <ToastProvider>')
  return api
}
