/**
 * Windows platform settings. docs/06 Phase 10.
 *
 * Notifications and run-at-login. Both are things the OS owns rather than the app, which is why
 * the read side asks the OS every time instead of caching: the user can change either from
 * Windows Settings, and a value we remembered would become a lie the panel repeats back.
 */

import { invoke, isTauri } from '@tauri-apps/api/core'

import type { NotifyPrefs } from './generated/NotifyPrefs'

const inTauri: boolean = isTauri()

const DEFAULT: NotifyPrefs = { enabled: true, vipOnly: false, sound: false }

export async function notifyPrefs(accountId: number): Promise<NotifyPrefs> {
  if (!inTauri) return DEFAULT
  return invoke<NotifyPrefs>('notify_prefs', { accountId })
}

export async function setNotifyPrefs(accountId: number, prefs: NotifyPrefs): Promise<void> {
  if (!inTauri) return
  await invoke('notify_set_prefs', { accountId, prefs })
}

export async function runAtLogin(): Promise<boolean> {
  if (!inTauri) return false
  return invoke<boolean>('run_at_login')
}

export async function setRunAtLogin(enabled: boolean): Promise<void> {
  if (!inTauri) return
  await invoke('set_run_at_login', { enabled })
}
